#pragma once
// Main window: toolbar, gallery (grid/list), bottom status bar.
#include "catalogstore.h"
#include "updatecheck.h"
#include <QComboBox>
#include <QLabel>
#include <QListView>
#include <QMainWindow>
#include <QPushButton>
#include <QStackedWidget>
#include <QStyledItemDelegate>
#include <QTimer>

class TileDelegate : public QStyledItemDelegate {
    Q_OBJECT
public:
    explicit TileDelegate(QObject *parent = nullptr);
    void paint(QPainter *p, const QStyleOptionViewItem &opt,
               const QModelIndex &idx) const override;
    QSize sizeHint(const QStyleOptionViewItem &opt,
                   const QModelIndex &idx) const override;
};

class MainWindow : public QMainWindow {
    Q_OBJECT
public:
    explicit MainWindow(QWidget *parent = nullptr);

private slots:
    void onSearch();
    void onReload();
    void onSettings();
    void onBrowseChanged(int i);
    void onSortChanged(int i);
    void onViewMode(bool grid);
    void onLibraryBack();
    void onTileActivated(const QModelIndex &idx);
    void onCounts(const QString &text);
    void onStatus(const QString &text, bool kobo, bool ok);
    void onLoading(bool loading);
    void onDbError(const QString &message);
    void showDbError(const QString &message);
    void onKoboOutcome(const QJsonObject &outcome);
    // Identify-then-ask: the engine returns ranked, deduped candidates and
    // the window picks between auto-open, a tie-break, and the
    // different-author offer. No candidate list is ever a dead end.
    void onKoboResolved(const QJsonObject &resolution);
    void onHandoffOffer(const QString &ip);
    void onDetailReady(const DetailItem &detail);
    void onHelp();
    void onAbout();
    void onChangelog();
    void onOpenDataFolder();
    void showContextMenu(const QPoint &pos);
    void saveGeometrySetting();

protected:
    void closeEvent(QCloseEvent *event) override;
    // Focus return is the natural moment to notice Calibre changed the
    // library; the probe is throttled inside the store.
    bool event(QEvent *e) override;
    void moveEvent(QMoveEvent *event) override;
    void resizeEvent(QResizeEvent *event) override;

private:
    void refreshSortBox();
    void restoreToolbar();
    void restoreGeometrySetting();
    QString currentView() const;
    void openDetail(int row);
    void openKoboFor(const BookItem &book);
    // Same title, another author on the device: sync stays primary, but the
    // other book is offered by name instead of being hidden.
    void offerDifferentAuthor(const QList<QJsonObject> &picks);
    // Genuine tie (same identity, two files) or "Choose…": pick one path.
    void showCandidateChooser(const QList<QJsonObject> &picks);

    CatalogStore m_store;
    UpdateChecker m_updater;
    QComboBox *m_browseBox;
    QComboBox *m_sortBox;
    QAction *m_backAct;
    QListView *m_grid;
    QListView *m_list;
    QStackedWidget *m_stack;
    QWidget *m_emptyPage;
    QWidget *m_loadingPage;
    QLabel *m_emptyTitle;
    QLabel *m_statusLabel;
    QAction *m_gridAct;
    QAction *m_listAct;
    BookItem m_pendingBook;
    int m_pendingRow = -1;
    DetailItem m_pendingDetail;
    bool m_detailLoading = false;
    bool m_koboActive = false;
    bool m_dbErrorOpen = false; // non-reentrant error dialog (load-chain guard)
    QTimer m_geomTimer; // debounced geometry persist (no settings churn)
};
