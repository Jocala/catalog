#pragma once
// CatalogStore: library data via blocking FFI, always off the UI thread.
// Modes mirror the other ports: Books / Authors / Series / Tags + drill-in.
#include "bookmodel.h"
#include "covercache.h"
#include "models.h"
#include "settings.h"
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

signals:
    void countsChanged(const QString &text);
    void statusChanged(const QString &text, bool kobo, bool ok);
    void loadingChanged(bool loading);
    void dbError(const QString &message);
    void koboOutcome(const QJsonObject &outcome);
    void searchDone(int bookCount, int seriesCount);

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

signals:
    void detailReady(const DetailItem &detail);

private slots:
    void onLoaded();
    void onCoverNeeded(const QString &path);
    void onCoverBatch(const QString &path, const QImage &img);
    void flushCovers();

private:
    struct LoadResult {
        QString kind; // books|authors|series|tags|search|detail
        QString payload;
        QString error;
    };
    void startLoad(const QString &kind, const QString &arg1 = QString());
    static LoadResult doLoad(QString cfg, QString kind, QString arg1, Sort sort,
                             Mode mode, QJsonObject searchParams);
    void applyLoad(const LoadResult &r);
    void fetchCover(const QString &path, int gen);
    QString statusCounts();
    // Default Kobo IP: explicit default wins; a single configured Kobo
    // is the default without starring.
    QString resolveKoboIp() const;

    AppSettings m_settings;
    BookModel m_model;
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
    QFutureWatcher<LoadResult> m_watcher;
};
