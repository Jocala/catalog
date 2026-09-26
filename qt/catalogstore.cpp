#include "catalogstore.h"
#include "ffi.h"
#include "ffijson.h"
#include "kobojob.h"
#include "version.h"
#include <QDateTime>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QPair>
// Capped load-lifecycle trace (startup-hang diagnostics): a few lines
// per launch in the temp dir. Per-cover chatter was removed for release.
#include <QDir>
#include <QFile>
#include <QTextStream>
#include <QTimer>
#include <QtConcurrent>
#include <chrono>
namespace {
int g_traceN = 0;
void trace(const QString &line) {
    if (g_traceN++ >= 2000) return;
    QFile f(QDir::tempPath() + "/qt-sort.txt");
    if (f.open(QIODevice::Append | QIODevice::Text)) {
        QTextStream s(&f);
        s << QDateTime::currentDateTime().toString("MM-dd hh:mm:ss.zzz ") << line << "\n";
    }
}
}

CatalogStore::CatalogStore(QObject *parent)
    : QObject(parent), m_diskCache(DiskCoverCache::defaultRoot()),
      m_log(ErrorLog::defaultRoot()) {
    m_settings = AppSettings::load();
    m_log.logHeader(kCatalogVersion, m_settings);
    connect(&m_watcher, &QFutureWatcher<LoadResult>::finished, this, &CatalogStore::onLoaded);
    connect(&m_model, &BookModel::coverNeeded, this, &CatalogStore::onCoverNeeded);
}

void CatalogStore::setSettings(const AppSettings &s) {
    m_settings = s;
}

void CatalogStore::setInitialView(Mode m, Sort s) {
    m_mode = m;
    m_sort = s;
}

void CatalogStore::saveView(const QString &viewMode) {
    m_settings.browseMode = (int)m_mode;
    m_settings.sortOrder = (int)m_sort;
    m_settings.viewMode = viewMode;
    AppSettings::save(m_settings);
}

void CatalogStore::reload(bool fresh) {
    exitSearch();
    m_drillKind.clear();
    m_freshNext = fresh;
    ++m_coverGen;
    startLoad("root");
}

void CatalogStore::setMode(Mode m) {
    m_mode = m;
    m_sort = ByAuthor;
    reload();
}

void CatalogStore::setSort(Sort s) {
    m_sort = s;
    ++m_coverGen;
    if (!m_drillKind.isEmpty() || m_searching)
        startLoad(m_searching ? "search" : "drill");
    else
        startLoad("root");
}

void CatalogStore::drillAuthor(qint64 id, const QString &title) {
    m_drillKind = "author";
    m_drillId = id;
    m_drillTitle = title;
    exitSearch();
    ++m_coverGen;
    startLoad("drill");
}

void CatalogStore::drillSeries(qint64 id, const QString &title) {
    m_drillKind = "series";
    m_drillId = id;
    m_drillTitle = title;
    exitSearch();
    ++m_coverGen;
    startLoad("drill");
}

void CatalogStore::drillTag(const QString &tag) {
    m_drillKind = "tag";
    m_drillTag = tag;
    exitSearch();
    ++m_coverGen;
    startLoad("drill");
}

void CatalogStore::exitDrill() {
    m_drillKind.clear();
}

bool CatalogStore::showingBooks() const {
    if (m_searching)
        return !m_searchSeriesMode;
    return !m_drillKind.isEmpty() || m_mode == Books;
}

void CatalogStore::exitSearch() {
    m_searching = false;
    m_searchSeriesMode = false;
}

void CatalogStore::runSearch(const QJsonObject &params) {
    m_searchParams = params;
    m_searching = true;
    m_searchSeriesMode = false;
    m_drillKind.clear();
    ++m_coverGen;
    startLoad("search");
}

void CatalogStore::requestDetail(qint64 id) {
    startLoad("detail", QString::number(id));
}

QString CatalogStore::resolveKoboIp() const {
    QString ip = m_settings.koboIp.trimmed();
    if (ip.isEmpty() && m_settings.koboIps.size() == 1)
        ip = m_settings.koboIps.first().trimmed();
    return ip;
}

void CatalogStore::openOnKobo(qint64 id, const QString &title, const QString &author) {
    QString ip = resolveKoboIp();
    if (ip.isEmpty()) {
        emit statusChanged("Kobo IP not set — enter it in Settings → Kobo", true, false);
        return;
    }
    if (m_settings.koboHandoffPromptDone || m_handoffBusy) {
        proceedOpenKobo(id, title, author, ip);
        return;
    }
    // First Read on Kobo: check the stacking fix before opening.
    m_pendingHandoff = {id, title, author, ip};
    m_handoffBusy = true;
    emit statusChanged("Checking Kobo stacking fix…", true, false);
    QFuture<QJsonObject> f = QtConcurrent::run([ip]() {
        return KoboJob::handoffCheck(ip);
    });
    QFutureWatcher<QJsonObject> *w = new QFutureWatcher<QJsonObject>(this);
    connect(w, &QFutureWatcher<QJsonObject>::finished, this,
            &CatalogStore::onHandoffChecked);
    w->setFuture(f);
}

void CatalogStore::markHandoffPromptDone() {
    m_settings.koboHandoffPromptDone = true;
    AppSettings::save(m_settings);
}

void CatalogStore::onHandoffChecked() {
    auto *w = static_cast<QFutureWatcher<QJsonObject> *>(sender());
    QJsonObject o = w ? w->result() : QJsonObject();
    if (w) w->deleteLater();
    HandoffOpen p = m_pendingHandoff;
    if (o.value("status").toString() == "failed" || o.value("code").toInt(-1) != 0) {
        // Can't verify (Kobo asleep? password-only login — the check is
        // key-only BatchMode). Don't nag; ask never again, just open.
        m_handoffBusy = false;
        markHandoffPromptDone();
        proceedOpenKobo(p.id, p.title, p.author, p.ip);
        return;
    }
    if (o.value("installed").toBool(false)) {
        m_handoffBusy = false;
        markHandoffPromptDone();
        proceedOpenKobo(p.id, p.title, p.author, p.ip);
        return;
    }
    emit handoffOffer(p.ip);
}

void CatalogStore::answerHandoff(bool install) {
    HandoffOpen p = m_pendingHandoff;
    if (!install) {
        m_handoffBusy = false;
        markHandoffPromptDone();
        proceedOpenKobo(p.id, p.title, p.author, p.ip);
        return;
    }
    emit statusChanged("Installing Kobo stacking fix…", true, false);
    QString ip = p.ip;
    QFuture<QJsonObject> f = QtConcurrent::run([ip]() {
        return KoboJob::handoffEnsure(ip);
    });
    QFutureWatcher<QJsonObject> *w = new QFutureWatcher<QJsonObject>(this);
    connect(w, &QFutureWatcher<QJsonObject>::finished, this,
            &CatalogStore::onHandoffEnsured);
    w->setFuture(f);
}

void CatalogStore::onHandoffEnsured() {
    auto *w = static_cast<QFutureWatcher<QJsonObject> *>(sender());
    QJsonObject o = w ? w->result() : QJsonObject();
    if (w) w->deleteLater();
    HandoffOpen p = m_pendingHandoff;
    m_handoffBusy = false;
    markHandoffPromptDone();
    QString state = o.value("state").toString();
    if (state == "installed" || state == "already") {
        emit statusChanged("Stacking fix installed", true, true);
    } else {
        QString msg = o.value("status").toString() == "failed"
            ? o.value("message").toString()
            : o.value("output").toString();
        emit statusChanged(QString("Stacking fix install failed: %1").arg(msg.left(120)), true, false);
    }
    proceedOpenKobo(p.id, p.title, p.author, p.ip);
}

void CatalogStore::proceedOpenKobo(qint64 id, const QString &title, const QString &author,
                                   const QString &ip) {
    emit statusChanged(QString("Opening “%1” on Kobo…").arg(title), true, false);
    AppSettings s = m_settings;
    QFuture<QJsonObject> f = QtConcurrent::run([s, ip, title, author, id]() {
        return KoboJob::open(s, ip, title, author, id);
    });
    QFutureWatcher<QJsonObject> *w = new QFutureWatcher<QJsonObject>(this);
    connect(w, &QFutureWatcher<QJsonObject>::finished, this, [this, w, title]() {
        QJsonObject o = w->result();
        w->deleteLater();
        QString status = o.value("status").toString();
        if (status == "opened") {
            emit statusChanged(QString("Opened on Kobo: %1").arg(title), true, true);
        } else if (status == "missing" || status == "ambiguous") {
            emit statusChanged(QString("Not on Kobo: %1").arg(title), true, false);
        } else {
            emit statusChanged(QString("Kobo failed: %1").arg(o.value("message").toString().left(160)), true, false);
        }
        emit koboOutcome(o);
    });
    w->setFuture(f);
}

void CatalogStore::syncOnKobo(qint64 id, const QString &title) {
    emit statusChanged(QString("Syncing “%1” to Kobo…").arg(title), true, false);
    AppSettings s = m_settings;
    QString ip = resolveKoboIp();
    QFuture<QJsonObject> f = QtConcurrent::run([s, ip, id]() {
        return KoboJob::sync(s, ip, id);
    });
    QFutureWatcher<QJsonObject> *w = new QFutureWatcher<QJsonObject>(this);
    connect(w, &QFutureWatcher<QJsonObject>::finished, this, [this, w, title]() {
        QJsonObject o = w->result();
        w->deleteLater();
        if (o.value("status").toString() == "opened")
            emit statusChanged(QString("Opened on Kobo: %1").arg(title), true, true);
        else
            emit statusChanged(QString("Kobo failed: %1").arg(o.value("message").toString().left(160)), true, false);
        emit koboOutcome(o);
    });
    w->setFuture(f);
}

void CatalogStore::startLoad(const QString &kind, const QString &arg1) {
    if (m_loading)
        return;
    m_loading = true;
    const int gen = ++m_loadGen;
    m_loadClock.start();
    trace(QString("load start gen=%1 kind=%2").arg(gen).arg(kind));
    emit loadingChanged(true);
    const bool freshUsed = m_freshNext;
    const bool diagUsed = m_settings.diagLogging;
    QString cfg = m_settings.libraryConfigJson(m_freshNext, diagUsed);
    m_freshNext = false;
    m_loadFresh = freshUsed;
    Sort sort = m_sort;
    Mode mode = m_mode;
    QJsonObject params = m_searchParams;
    QString drillKind = m_drillKind;
    qint64 drillId = m_drillId;
    QString drillTag = m_drillTag;
    QString arg = arg1;
    if (kind == "drill") {
        if (drillKind == "author")
            arg = QString("author_books:%1").arg(drillId);
        else if (drillKind == "series")
            arg = QString("series_books:%1").arg(drillId);
        else if (drillKind == "tag")
            arg = drillTag;
    }
    const auto t0 = std::chrono::steady_clock::now();
    QFuture<LoadResult> f = QtConcurrent::run([=]() {
        const auto tEntry = std::chrono::steady_clock::now();
        LoadResult r = doLoad(cfg, kind, arg, sort, mode, params, diagUsed);
        r.gen = gen;
        r.fresh = freshUsed;
        r.diag = diagUsed;
        r.queueMs = std::chrono::duration_cast<std::chrono::milliseconds>(tEntry - t0).count();
        return r;
    });
    m_watcher.setFuture(f);
    // Watchdog: the FFI bounds only the metadata fetch (30s); stages
    // past it have no deadline. If the worker never returns, orphan the
    // generation so a late result can't resurrect it, and hand the user
    // a retry instead of a permanent Loading… page.
    QTimer::singleShot(LOAD_TIMEOUT_MS, this, [this, gen, kind]() { onLoadTimeout(gen, kind); });
}

CatalogStore::LoadResult CatalogStore::doLoad(QString cfg, QString kind, QString arg1,
                                              Sort sort, Mode mode, QJsonObject searchParams,
                                              bool diag) {
    LoadResult r;
    r.kind = kind;
    QByteArray cfgB = cfg.toUtf8();
    bool desc = (sort == ZA || sort == Oldest);
    bool byAuthor = (sort == ByAuthor);
    bool byDate = (sort == Newest || sort == Oldest);
    QJsonValue payload;
    QString err;
    char *raw = nullptr;
    QElapsedTimer ffiClock;
    ffiClock.start();
    if (kind == "root") {
        if (mode == Books) {
            raw = catalog_fetch_books(cfgB.constData(), "", desc, byAuthor, byDate);
        } else if (mode == Authors) {
            raw = catalog_browse(cfgB.constData(), "authors", desc, false);
        } else if (mode == Series) {
            raw = catalog_browse(cfgB.constData(), "series", desc, byAuthor);
        } else {
            raw = catalog_browse(cfgB.constData(), "tags", desc, false);
        }
    } else if (kind == "drill") {
        if (!arg1.startsWith("author_books:") && !arg1.startsWith("series_books:")) {
            // Tag drill: search by tag (browse has no tag drill mode).
            QJsonObject p;
            p.insert("tag", arg1);
            p.insert("sort_descending", desc);
            QByteArray pb = QJsonDocument(p).toJson(QJsonDocument::Compact);
            raw = catalog_search(cfgB.constData(), pb.constData());
        } else {
            raw = catalog_browse(cfgB.constData(), arg1.toUtf8().constData(), desc, byAuthor);
        }
    } else if (kind == "search") {
        // Tag→series expand (mirrors SearchForm::is_tag_expand): a tag-only
        // search with the box checked browses series containing the tag
        // instead of books carrying it.
        QString tag = searchParams.value("tag").toString().trimmed();
        bool expand = searchParams.value("expand_tag").toBool(false);
        bool tagOnly = expand && !tag.isEmpty()
            && searchParams.value("query").toString().trimmed().isEmpty()
            && searchParams.value("title").toString().trimmed().isEmpty()
            && searchParams.value("author").toString().trimmed().isEmpty()
            && searchParams.value("series").toString().trimmed().isEmpty();
        if (tagOnly) {
            QString mode = QString("tag_series:") + tag;
            raw = catalog_browse(cfgB.constData(), mode.toUtf8().constData(), desc, byAuthor);
            if (raw)
                r.kind = "search-series";
        } else {
            searchParams.insert("sort_descending", desc);
            QByteArray pb = QJsonDocument(searchParams).toJson(QJsonDocument::Compact);
            raw = catalog_search(cfgB.constData(), pb.constData());
        }
    } else if (kind == "detail") {
        raw = catalog_detail(cfgB.constData(), arg1.toLongLong());
    }
    r.ffiMs = ffiClock.elapsed();
    if (diag) {
        // Same worker thread, immediately after the FFI call that stashed
        // it (covers never touch the diag slot, loads are serialized —
        // except an orphaned watchdog worker, whose diag may rarely
        // interleave; acceptable for diagnostics).
        QJsonValue diagPayload;
        QString diagErr;
        if (ffiOk(catalog_last_diag(), diagPayload, diagErr) && diagPayload.isObject()) {
            QJsonObject d = diagPayload.toObject();
            r.fetch1Ms = (qint64)d.value("fetch1_ms").toDouble(0);
            r.fetch2Ms = (qint64)d.value("fetch2_ms").toDouble(0);
            r.openMs = (qint64)d.value("open_ms").toDouble(0);
            r.queryMs = (qint64)d.value("query_ms").toDouble(0);
            r.dbBytes = (qint64)d.value("bytes").toDouble(0);
            r.attempts = d.value("attempts").toInt(0);
            r.pooled = d.value("pooled").toString();
            r.fetch1Pooled = d.value("fetch1_pooled").toString();
            r.fetch1ConnectMs = (qint64)d.value("fetch1_connect_ms").toDouble(0);
            r.fetch1ReadMs = (qint64)d.value("fetch1_read_ms").toDouble(0);
            r.fetch1EvictN = d.value("fetch1_evict_n").toInt(0);
            r.evictN = d.value("evict_n").toInt(0);
            r.cached = d.value("cached").toBool(false);
            r.chunksDone = (qint64)d.value("chunks_done").toDouble(0);
            r.chunksTotal = (qint64)d.value("chunks_total").toDouble(0);
            r.fetch1ChunksDone = (qint64)d.value("fetch1_chunks_done").toDouble(0);
            r.fetch1ChunksTotal = (qint64)d.value("fetch1_chunks_total").toDouble(0);
            r.copyHit = d.value("copy_hit").toBool(false);
            r.copyStale = d.value("copy_stale").toBool(false);
            r.copyMs = (qint64)d.value("copy_ms").toDouble(0);
            r.copyAgeSecs = (qint64)d.value("copy_age_secs").toDouble(0);
        }
    }
    if (!raw) {
        r.error = "ffi returned null";
        return r;
    }
    QElapsedTimer parseClock;
    parseClock.start();
    if (!ffiOk(raw, payload, err)) {
        r.error = err;
        r.parseMs = parseClock.elapsed();
        return r;
    }
    r.payload = QString::fromUtf8(
        QJsonDocument::fromVariant(payload.toVariant()).toJson(QJsonDocument::Compact));
    r.parseMs = parseClock.elapsed();
    return r;
}

void CatalogStore::onLoaded() {
    LoadResult r = m_watcher.result();
    if (r.gen != m_loadGen) {
        trace(QString("loaded STALE gen=%1 cur=%2 kind=%3").arg(r.gen).arg(m_loadGen).arg(r.kind));
        return; // orphaned by the watchdog (latch already cleared there)
    }
    m_loading = false;
    trace(QString("loaded gen=%1 kind=%2 ms=%3").arg(r.gen).arg(r.kind).arg(m_loadClock.elapsed()));
    // Order matters: the page decision (loadingChanged -> rowCount check)
    // must see the filled model. Emitting first locks a fresh launch onto
    // the empty page even when books arrived — the grid then never shows
    // until the next reload finds stale rows.
    QElapsedTimer applyClock;
    applyClock.start();
    applyLoad(r);
    const qint64 applyMs = applyClock.elapsed();
    const qint64 totalMs = m_loadClock.elapsed();
    if (m_settings.diagLogging) {
        const int rows = (r.kind == "detail") ? (r.error.isEmpty() ? 1 : 0) : m_model.rowCount();
        QString msg = QString("gen=%1 kind=%2 fresh=%3 queue_ms=%4 ffi_ms=%5 parse_ms=%6 apply_ms=%7 total_ms=%8 rows=%9")
                          .arg(r.gen)
                          .arg(r.kind)
                          .arg(r.fresh ? 1 : 0)
                          .arg(r.queueMs)
                          .arg(r.ffiMs)
                          .arg(r.parseMs)
                          .arg(applyMs)
                          .arg(totalMs)
                          .arg(rows);
        if (r.diag) {
            msg += QString(" fetch1_ms=%1 fetch2_ms=%2 open_ms=%3 query_ms=%4 attempts=%5 pooled=%6 bytes=%7 cached=%8 evict_n=%9 fetch1_pooled=%10 fetch1_conn=%11 fetch1_read=%12 fetch1_evict=%13 chunks=%14/%15 fetch1_chunks=%16/%17 copy_hit=%18 copy_stale=%19 copy_ms=%20 copy_age=%21")
                       .arg(r.fetch1Ms)
                       .arg(r.fetch2Ms)
                       .arg(r.openMs)
                       .arg(r.queryMs)
                       .arg(r.attempts)
                       .arg(r.pooled.isEmpty() ? QString("-") : r.pooled)
                       .arg(r.dbBytes)
                       .arg(r.cached ? 1 : 0)
                       .arg(r.evictN)
                       .arg(r.fetch1Pooled.isEmpty() ? QString("-") : r.fetch1Pooled)
                       .arg(r.fetch1ConnectMs)
                       .arg(r.fetch1ReadMs)
                       .arg(r.fetch1EvictN)
                       .arg(r.chunksDone)
                       .arg(r.chunksTotal)
                       .arg(r.fetch1ChunksDone)
                       .arg(r.fetch1ChunksTotal)
                       .arg(r.copyHit ? 1 : 0)
                       .arg(r.copyStale ? 1 : 0)
                       .arg(r.copyMs)
                       .arg(r.copyAgeSecs);
        }
        if (!r.error.isEmpty())
            msg += " error=" + r.error.left(160);
        m_log.log("diag", msg);
    }
    emit loadingChanged(false);
}

void CatalogStore::onLoadTimeout(int gen, const QString &kind) {
    if (gen != m_loadGen || !m_loading)
        return;
    trace(QString("load TIMEOUT gen=%1 kind=%2 ms=%3").arg(gen).arg(kind).arg(m_loadClock.elapsed()));
    ++m_loadGen; // orphan the stuck worker; its late result dies in onLoaded
    m_loading = false;
    emit loadingChanged(false);
    if (kind == "detail") {
        emit statusChanged("Book detail timed out — try again", false, false);
        return;
    }
    const QString msg =
        "Could not reach the Calibre library (load timed out - the library may be unreachable).";
    m_log.log("library", QString("kind=%1 gen=%2 ms=%3 fresh=%4 :: %5")
                               .arg(kind).arg(gen).arg(m_loadClock.elapsed())
                               .arg(m_loadFresh ? 1 : 0).arg(msg));
    emit dbError(msg);
}

void CatalogStore::applyLoad(const LoadResult &r) {
    if (!r.error.isEmpty()) {
        trace(QString("applyLoad kind=%1 ERROR %2").arg(r.kind).arg(r.error.left(100)));
        m_log.log("library", QString("kind=%1 gen=%2 ms=%3 :: %4")
                                     .arg(r.kind).arg(r.gen).arg(m_loadClock.elapsed()).arg(r.error));
        emit dbError(r.error);
        return;
    }
    QJsonParseError perr;
    QJsonDocument doc = QJsonDocument::fromJson(r.payload.toUtf8(), &perr);
    if (perr.error != QJsonParseError::NoError)
        return;
    QJsonValue payload = doc.isArray() ? QJsonValue(doc.array()) : QJsonValue(doc.object());
    if (r.kind == "detail") {
        emit detailReady(DetailItem::fromJson(payload.toObject()));
        return;
    }
    if (r.kind == "root") {
        if (m_mode == Books)
            m_model.setBooks(parseBooks(payload));
        else if (m_mode == Authors) {
            QStringList t, s;
            QList<qint64> ids;
            QStringList covers;
            for (const AuthorItem &a : parseAuthors(payload)) {
                t << a.name;
                s << a.bookCount;
                ids << a.id;
                covers << a.firstPath;
            }
            m_model.setTiles(t, s, ids, covers);
        } else if (m_mode == Series) {
            QStringList t, s;
            QList<qint64> ids;
            QStringList covers;
            for (const SeriesItem &x : parseSeries(payload)) {
                t << x.name;
                s << x.bookCount;
                ids << x.id;
                covers << x.firstPath;
            }
            m_model.setTiles(t, s, ids, covers);
        } else {
            QStringList t, s;
            QList<qint64> ids;
            QStringList covers;
            for (const TagItem &x : parseTags(payload)) {
                t << x.name;
                s << x.bookCount;
                ids << x.id;
                covers << QString();
            }
            m_model.setTiles(t, s, ids, covers, true);
        }
    } else if (r.kind == "drill" || r.kind == "search" || r.kind == "search-series") {
        if (r.kind == "search-series") {
            QStringList t, s;
            QList<qint64> ids;
            QStringList covers;
            for (const SeriesItem &x : parseSeries(payload)) {
                t << x.name;
                s << x.bookCount;
                ids << x.id;
                covers << x.firstPath;
            }
            m_searchSeries = t.size();
            m_searchSeriesMode = true;
            m_model.setTiles(t, s, ids, covers);
        } else {
            QList<BookItem> books = parseBooks(payload);
            if (r.kind == "search") {
                m_searchBooks = books.size();
                m_searchSeriesMode = false;
            }
            m_model.setBooks(books);
        }
    }
    emit countsChanged(statusCounts());
}

QString CatalogStore::statusCounts() {
    if (m_searching) {
        if (m_searchSeriesMode)
            return QString("%1 series").arg(m_searchSeries);
        return QString("%1 results").arg(m_searchBooks);
    }
    if (!m_drillKind.isEmpty())
        return QString("%1 books").arg(m_model.rowCount());
    switch (m_mode) {
    case Books:
        return QString("%1 books").arg(m_model.rowCount());
    case Authors:
        return QString("%1 authors").arg(m_model.rowCount());
    case Series:
        return QString("%1 series").arg(m_model.rowCount());
    case Tags:
        return QString("%1 tags").arg(m_model.rowCount());
    }
    return QString();
}

void CatalogStore::onCoverNeeded(const QString &path) {
    if (path.isEmpty() || m_inFlight.contains(path)) {
        return;
    }
    // Memory hit: the model may have been reset (sort/mode/search) since
    // this arrived, so re-deliver to the model instead of assuming it is
    // already there. (Previously this returned silently — every cover
    // re-shown after a reset stayed blank with no refetch.)
    auto mem = m_coverCache.find(path);
    if (mem != m_coverCache.end()) {
        onCoverBatch(path, *mem);
        return;
    }
    // Disk first: warm starts never touch SMB for covers (small local
    // read, fine on the UI thread).
    QByteArray jpg;
    if (m_diskCache.tryGet(path, jpg)) {
        QImage img;
        if (img.loadFromData(jpg)) {
            onCoverBatch(path, img);
            return;
        }
    }
    if (m_coverActive >= COVER_MAX) {
        return; // coalesce: the path re-demands on its next realize
    }
    fetchCover(path, m_coverGen);
}

void CatalogStore::fetchCover(const QString &path, int gen) {
    m_inFlight.insert(path);
    ++m_coverActive;
    QString cfg = m_settings.libraryConfigJson();
    QFuture<QPair<QString, QImage>> f = QtConcurrent::run([this, cfg, path]() -> QPair<QString, QImage> {
        QByteArray cfgB = cfg.toUtf8();
        QByteArray pB = path.toUtf8();
        QByteArray bytes = ffiBytes(catalog_cover(cfgB.constData(), pB.constData(), 187, 240));
        if (!bytes.isEmpty())
            m_diskCache.put(path, bytes);
        QImage img;
        if (!bytes.isEmpty())
            img.loadFromData(bytes);
        return qMakePair(path, img);
    });
    QFutureWatcher<QPair<QString, QImage>> *w = new QFutureWatcher<QPair<QString, QImage>>(this);
    connect(w, &QFutureWatcher<QPair<QString, QImage>>::finished, this, [this, w, gen]() {
        auto r = w->result();
        w->deleteLater();
        m_inFlight.remove(r.first);
        --m_coverActive;
        if (gen != m_coverGen)
            return; // superseded load: keep the disk bytes, skip the paint
        onCoverBatch(r.first, r.second);
    });
    w->setFuture(f);
}

void CatalogStore::onCoverBatch(const QString &path, QImage img) {
    if (img.isNull()) {
        return;
    }
    auto it = m_coverCache.find(path);
    if (it != m_coverCache.end()) {
        m_coverBytes -= it->sizeInBytes();
        m_coverCache.erase(it);
        m_coverOrder.removeAll(path);
    }
    m_coverCache.insert(path, img);
    m_coverOrder.append(path);
    m_coverBytes += img.sizeInBytes();
    while (m_coverBytes > COVER_MAX_BYTES && !m_coverOrder.isEmpty()) {
        QString old = m_coverOrder.takeFirst();
        auto jt = m_coverCache.find(old);
        if (jt != m_coverCache.end()) {
            m_coverBytes -= jt->sizeInBytes();
            m_coverCache.erase(jt);
        }
    }
    m_coverPending.append(qMakePair(path, img));
    if (!m_coverFlushScheduled) {
        m_coverFlushScheduled = true;
        QTimer::singleShot(120, this, &CatalogStore::flushCovers);
    }
}

void CatalogStore::flushCovers() {
    m_coverFlushScheduled = false;
    if (m_coverPending.isEmpty())
        return;
    m_model.setCoverBatch(m_coverPending);
    m_coverPending.clear();
}
