#pragma once
// CatalogStore: library data via blocking FFI, always off the UI thread.
// Modes mirror the other ports: Books / Authors / Series / Tags + drill-in.
#include "bookmodel.h"
#include "covercache.h"
#include "errorlog.h"
#include "models.h"
#include "settings.h"
#include <QElapsedTimer>
#include <QFutureWatcher>
#include <QObject>
#include <QSet>

class CatalogStore : public QObject {
    Q_OBJECT
public:
    enum Mode { Books, Authors, Series, Tags };
    enum Sort { ByAuthor, AZ, ZA, Newest, Oldest };

    explicit CatalogStore(QObject *parent = nullptr);

    BookModel *model() { return &m_model; }
    AppSettings settings() const { return m_settings; }
    void setSettings(const AppSettings &s);

    Mode mode() const { return m_mode; }
    Sort sort() const { return m_sort; }
    bool showingBooks() const;
    // Search tag→series expand shows series tiles, not books.
    bool searchSeriesMode() const { return m_searching && m_searchSeriesMode; }
    // Startup restore: set toolbar state without starting a load
    // (the constructor's reload() does the single load).
    void setInitialView(Mode m, Sort s);
    // Persist toolbar state (browse/sort from the store, view from the window).
    void saveView(const QString &viewMode);

signals:
    void countsChanged(const QString &text);
    void statusChanged(const QString &text, bool kobo, bool ok);
    void loadingChanged(bool loading);
    void dbError(const QString &message);
    void koboOutcome(const QJsonObject &outcome);
    void searchDone(int bookCount, int seriesCount);
    // First-run stacking-fix offer: MainWindow asks Install / Not Now,
    // then calls answerHandoff. Emitted off the load path, never modal here.
    void handoffOffer(const QString &ip);

public slots:
    void reload(bool fresh = false);
    void setMode(Mode m);
    void setSort(Sort s);
    void drillAuthor(qint64 id, const QString &title);
    void drillSeries(qint64 id, const QString &title);
    void drillTag(const QString &tag);
    void exitDrill();
    void exitSearch();
    void runSearch(const QJsonObject &params);
    void requestDetail(qint64 id);
    void openOnKobo(qint64 id, const QString &title, const QString &author);
    void syncOnKobo(qint64 id, const QString &title);
    // Answer to handoffOffer: install the fix, then open; otherwise just open.
    void answerHandoff(bool install);

signals:
    void detailReady(const DetailItem &detail);

private slots:
    void onLoaded();
    void onLoadTimeout(int gen, const QString &kind);
    void onHandoffChecked();
    void onHandoffEnsured();
    void onCoverNeeded(const QString &path);
    void onCoverBatch(const QString &path, QImage img);
    void flushCovers();

private:
    struct LoadResult {
        QString kind; // books|authors|series|tags|search|detail
        QString payload;
        QString error;
        int gen = 0; // startLoad generation: stale results are discarded
        // Diag splits (ms): queue = startLoad->worker entry, ffi = single
        // blocking engine call, parse = ffi JSON -> compact payload,
        // fetch1/2 = engine metadata attempts, open = :memory: load,
        // query = SQL after open. Fetch-side fields come from
        // catalog_last_diag() and are only filled when the load was
        // diag-gated (Settings → Diagnostic logging); otherwise zero.
        qint64 queueMs = 0;
        qint64 ffiMs = 0;
        qint64 parseMs = 0;
        qint64 fetch1Ms = 0;
        qint64 fetch2Ms = 0;
        qint64 openMs = 0;
        qint64 queryMs = 0;
        qint64 dbBytes = 0;
        int attempts = 0;
        QString pooled;
        // Attempt-1 split (the interesting half of a stall) + eviction
        // counts + cache-hit marker. Zero/false when not diag-gated.
        QString fetch1Pooled;
        qint64 fetch1ConnectMs = 0;
        qint64 fetch1ReadMs = 0;
        int fetch1EvictN = 0;
        int evictN = 0;
        bool cached = false;
        // Chunk progress (1 MiB chunks): completed / total, this attempt
        // and attempt 1. Settles wedge-vs-crawl on the next stall.
        qint64 chunksDone = 0;
        qint64 chunksTotal = 0;
        qint64 fetch1ChunksDone = 0;
        qint64 fetch1ChunksTotal = 0;
        bool fresh = false; // Reload bypass of the FFI metadata cache
        bool diag = false;  // extended fetch timings requested
    };
    void startLoad(const QString &kind, const QString &arg1 = QString());
    static LoadResult doLoad(QString cfg, QString kind, QString arg1, Sort sort,
                             Mode mode, QJsonObject searchParams, bool diag);
    void applyLoad(const LoadResult &r);
    void fetchCover(const QString &path, int gen);
    QString statusCounts();
    // Default Kobo IP: explicit default wins; a single configured Kobo
    // is the default without starring.
    QString resolveKoboIp() const;

    AppSettings m_settings;
    BookModel m_model;
    ErrorLog m_log;
    Mode m_mode = Books;
    Sort m_sort = ByAuthor;
    bool m_loading = false;
    bool m_freshNext = false;
    bool m_searching = false;
    // Drill state: author/series id + title, or tag name.
    QString m_drillKind;
    qint64 m_drillId = -1;
    QString m_drillTitle;
    QString m_drillTag;
    QJsonObject m_searchParams;
    int m_searchBooks = 0;
    // Tag→series expand state: a tag-only search with expand shows a
    // series grid (drillable) instead of the book grid.
    bool m_searchSeriesMode = false;
    int m_searchSeries = 0;
    QSet<QString> m_inFlight;
    QHash<QString, QImage> m_coverCache;
    DiskCoverCache m_diskCache;
    // Memory bound by BYTES, not count: a full library (~135 MB of small
    // covers) sits far below the cap, so eviction practically never fires
    // and a settled view can never lose a visible cover to a random
    // victim (the frozen-gap bug). Oldest-inserted goes first when it
    // does fire.
    QList<QString> m_coverOrder;
    qint64 m_coverBytes = 0;
    static const qint64 COVER_MAX_BYTES = 256LL * 1024 * 1024;
    // Cover fetch gate (mirrors the C#/Swift 6-slot throttler): at most
    // COVER_MAX concurrent SMB fetches. Overflow used to queue without
    // limit — a fast cold scroll backloged hundreds of stale fetches and
    // repaints. Now there is no queue: a skipped path simply re-demands
    // on its next realize (demand coalescing, same as the WinUI port).
    // m_coverGen drops completions from superseded loads (browse/sort
    // switches never clear in-flight workers, but their results die).
    int m_coverActive = 0;
    int m_coverGen = 0;
    static const int COVER_MAX = 6;
    // Coalesced paint updates: fetched covers accumulate here and flush
    // to the model on a 120ms tick (one layout pass per batch).
    QList<QPair<QString, QImage>> m_coverPending;
    bool m_coverFlushScheduled = false;
    // First-run handoff gate: the open stashed here while the
    // check/offer/install chain runs. Asked once per installation.
    struct HandoffOpen { qint64 id = -1; QString title; QString author; QString ip; };
    HandoffOpen m_pendingHandoff;
    bool m_handoffBusy = false;
    void proceedOpenKobo(qint64 id, const QString &title, const QString &author,
                         const QString &ip);
    void markHandoffPromptDone();
    QFutureWatcher<LoadResult> m_watcher;
    // Load watchdog: startLoad latches m_loading until onLoaded. If the
    // worker never returns (some FFI stages have no deadline), the window
    // would sit on Loading… forever and every later reload() would no-op.
    // The watchdog orphans the stuck generation and raises an error so
    // the user gets Retry instead of a force-quit. Late results from
    // orphaned generations are discarded in onLoaded.
    int m_loadGen = 0;
    QElapsedTimer m_loadClock;
    bool m_loadFresh = false; // fresh flag of the in-flight load (watchdog line)
    static const int LOAD_TIMEOUT_MS = 45000;
};
