#include "catalogstore.h"
#include "ffi.h"
#include "ffijson.h"
#include "kobojob.h"
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QPair>
// TEMP: capped trace for sort-change gaps (revert before merge).
#include <QDir>
#include <QFile>
#include <QTextStream>
#include <QTimer>
#include <QtConcurrent>
namespace {
int g_traceN = 0;
void trace(const QString &line) {
    if (g_traceN++ >= 5000) return;
    QFile f(QDir::tempPath() + "/qt-sort.txt");
    if (f.open(QIODevice::Append | QIODevice::Text)) {
        QTextStream s(&f);
        s << line << "\n";
    }
}
}

CatalogStore::CatalogStore(QObject *parent)
    : QObject(parent), m_diskCache(DiskCoverCache::defaultRoot()) {
    m_settings = AppSettings::load();
    connect(&m_watcher, &QFutureWatcher<LoadResult>::finished, this, &CatalogStore::onLoaded);
    connect(&m_model, &BookModel::coverNeeded, this, &CatalogStore::onCoverNeeded);
}

void CatalogStore::setSettings(const AppSettings &s) {
    m_settings = s;
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
    trace(QString("setsort gen=%1").arg(m_coverGen));
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
    return m_searching || !m_drillKind.isEmpty() || m_mode == Books;
}

void CatalogStore::exitSearch() {
    m_searching = false;
}

void CatalogStore::runSearch(const QJsonObject &params) {
    m_searchParams = params;
    m_searching = true;
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
    emit statusChanged(QString("Opening “%1” on Kobo…").arg(title), true, false);
    AppSettings s = m_settings;
    QString ip = resolveKoboIp();
    if (ip.isEmpty()) {
        emit statusChanged("Kobo IP not set — enter it in Settings → Kobo", true, false);
        return;
    }
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
    emit loadingChanged(true);
    QString cfg = m_settings.libraryConfigJson(m_freshNext);
    m_freshNext = false;
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
    QFuture<LoadResult> f = QtConcurrent::run([=]() {
        return doLoad(cfg, kind, arg, sort, mode, params);
    });
    m_watcher.setFuture(f);
}

CatalogStore::LoadResult CatalogStore::doLoad(QString cfg, QString kind, QString arg1,
                                              Sort sort, Mode mode, QJsonObject searchParams) {
    LoadResult r;
    r.kind = kind;
    QByteArray cfgB = cfg.toUtf8();
    bool desc = (sort == ZA || sort == Oldest);
    bool byAuthor = (sort == ByAuthor);
    bool byDate = (sort == Newest || sort == Oldest);
    QJsonValue payload;
    QString err;
    char *raw = nullptr;
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
        searchParams.insert("sort_descending", desc);
        QByteArray pb = QJsonDocument(searchParams).toJson(QJsonDocument::Compact);
        raw = catalog_search(cfgB.constData(), pb.constData());
    } else if (kind == "detail") {
        raw = catalog_detail(cfgB.constData(), arg1.toLongLong());
    }
    if (!raw) {
        r.error = "ffi returned null";
        return r;
    }
    if (!ffiOk(raw, payload, err)) {
        r.error = err;
        return r;
    }
    r.payload = QString::fromUtf8(
        QJsonDocument::fromVariant(payload.toVariant()).toJson(QJsonDocument::Compact));
    return r;
}

void CatalogStore::onLoaded() {
    m_loading = false;
    // Order matters: the page decision (loadingChanged -> rowCount check)
    // must see the filled model. Emitting first locks a fresh launch onto
    // the empty page even when books arrived — the grid then never shows
    // until the next reload finds stale rows.
    applyLoad(m_watcher.result());
    emit loadingChanged(false);
}

void CatalogStore::applyLoad(const LoadResult &r) {
    if (!r.error.isEmpty()) {
        trace(QString("applyLoad kind=%1 ERROR %2").arg(r.kind).arg(r.error.left(100)));
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
            m_model.setTiles(t, s, ids, covers);
        }
    } else if (r.kind == "drill" || r.kind == "search") {
        QList<BookItem> books = parseBooks(payload);
        if (r.kind == "search")
            m_searchBooks = books.size();
        m_model.setBooks(books);
    }
    emit countsChanged(statusCounts());
}

QString CatalogStore::statusCounts() {
    if (m_searching)
        return QString("%1 results").arg(m_searchBooks);
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
        trace(QString("need skip gen=%1 active=%2 %3").arg(m_coverGen).arg(m_coverActive).arg(path));
        return;
    }
    // Memory hit: the model may have been reset (sort/mode/search) since
    // this arrived, so re-deliver to the model instead of assuming it is
    // already there. (Previously this returned silently — every cover
    // re-shown after a reset stayed blank with no refetch.)
    auto mem = m_coverCache.find(path);
    if (mem != m_coverCache.end()) {
        trace(QString("need memhit gen=%1 %2").arg(m_coverGen).arg(path));
        onCoverBatch(path, *mem);
        return;
    }
    // Disk first: warm starts never touch SMB for covers (small local
    // read, fine on the UI thread).
    QByteArray jpg;
    if (m_diskCache.tryGet(path, jpg)) {
        QImage img;
        if (img.loadFromData(jpg)) {
            trace(QString("need diskhit gen=%1 %2").arg(m_coverGen).arg(path));
            onCoverBatch(path, img);
            return;
        }
        trace(QString("need diskcorrupt gen=%1 %2").arg(m_coverGen).arg(path));
    }
    if (m_coverActive >= COVER_MAX) {
        trace(QString("need coalesce gen=%1 active=%2 %3").arg(m_coverGen).arg(m_coverActive).arg(path));
        return; // coalesce: the path re-demands on its next realize
    }
    trace(QString("need fetch gen=%1 %2").arg(m_coverGen).arg(path));
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
        trace(QString("done null=%1 gen=%2 cur=%3 %4").arg(r.second.isNull()).arg(gen).arg(m_coverGen).arg(r.first));
        if (gen != m_coverGen)
            return; // superseded load: keep the disk bytes, skip the paint
        onCoverBatch(r.first, r.second);
    });
    w->setFuture(f);
}

void CatalogStore::onCoverBatch(const QString &path, QImage img) {
    if (img.isNull()) {
        trace(QString("batch nullimg %1").arg(path));
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
