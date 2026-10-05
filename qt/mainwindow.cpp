#include "mainwindow.h"
#include "aboutdialog.h"
#include "bookinfodialog.h"
#include "helpdialog.h"
#include "listdelegate.h"
#include "tagicon.h"
#include "searchdialog.h"
#include "settingsdialog.h"
#include "theme.h"
#include <QAction>
#include <QApplication>
#include <QCloseEvent>
#include <QCoreApplication>
#include <QDesktopServices>
#include <QDialog>
#include <QDir>
#include <QFileInfo>
#include <QGuiApplication>
#include <QHBoxLayout>
#include <QMenu>
#include <QMenuBar>
#include <QMessageBox>
#include <QMoveEvent>
#include <QPainter>
#include <QProcess>
#include <QProgressBar>
#include <QPushButton>
#include <QResizeEvent>
#include <QScreen>
#include <QSizePolicy>
#include <QStandardPaths>
#include <QStatusBar>
#include <QToolBar>
#include <QToolButton>
#include <QUrl>
#include <QVBoxLayout>

TileDelegate::TileDelegate(QObject *parent) : QStyledItemDelegate(parent) {}

static constexpr int kTileW = 200;
static constexpr int kTileH = 300;
static constexpr int kColumns = 4;

QSize TileDelegate::sizeHint(const QStyleOptionViewItem &, const QModelIndex &) const {
    return QSize(kTileW, kTileH);
}

void TileDelegate::paint(QPainter *p, const QStyleOptionViewItem &opt,
                         const QModelIndex &idx) const {
    p->save();
    // No selection fill: a clicked book looks identical to an unclicked
    // one (house rule). Hover tint stays as the position cue.
    if (opt.state & QStyle::State_MouseOver) {
        p->fillRect(opt.rect, opt.palette.alternateBase());
    }
    QRect coverRect(opt.rect.x() + 6, opt.rect.y() + 4, 187, 240);
    if (idx.data(BookModel::TagTileRole).toBool()) {
        // Tags are category marks, not books: use the same glyph in the
        // tile position instead of asking the cover pipeline for a path.
        paintTagIcon(p, coverRect);
    } else {
        QPixmap pm = idx.data(BookModel::CoverRole).value<QPixmap>();
        if (pm.isNull()) {
            p->fillRect(coverRect, QColor(0, 0, 0, 20));
            p->setPen(Qt::gray);
            p->drawText(coverRect, Qt::AlignCenter, "No cover");
        } else {
            // Aspect-fit: source aspects vary (the engine fits within the
            // fetch box), so a raw drawPixmap stretch distorts. Scale into
            // the box, center, gutter the remainder.
            QSize fitted = pm.size().scaled(coverRect.size(), Qt::KeepAspectRatio);
            QPoint at(coverRect.center().x() - fitted.width() / 2,
                      coverRect.center().y() - fitted.height() / 2);
            p->fillRect(coverRect, QColor(0, 0, 0, 20));
            p->drawPixmap(QRect(at, fitted), pm);
        }
    }
    QRect titleRect(opt.rect.x(), opt.rect.y() + 246, 200, 34);
    p->setPen(opt.palette.text().color());
    QFont f = p->font();
    f.setPointSize(9);
    p->setFont(f);
    QString shown = displayTitle(idx.data(BookModel::TitleRole).toString(),
                                 idx.data(BookModel::SeriesRole).toString(),
                                 idx.data(BookModel::SeriesIndexRole).toDouble());
    p->drawText(titleRect, Qt::AlignHCenter | Qt::TextWordWrap, shown);
    QRect subRect(opt.rect.x(), opt.rect.y() + 282, 200, 16);
    p->setPen(Qt::gray);
    f.setPointSize(8);
    p->setFont(f);
    QString sub = idx.data(BookModel::AuthorRole).toString();
    p->drawText(subRect, Qt::AlignHCenter,
                opt.fontMetrics.elidedText(sub, Qt::ElideRight, 196));
    p->restore();
}

MainWindow::MainWindow(QWidget *parent) : QMainWindow(parent) {
    setWindowTitle("Jocala Catalog");
    // Fixed width: hug exactly kColumns gallery tiles (no 3- or 5-across).
    // Height stays resizable; the reserve for a vertical scrollbar keeps
    // 4-across stable whether the bar is shown or not.
    const int spacing = 12;
    const int sb = style()->pixelMetric(QStyle::PM_ScrollBarExtent);
    const int gridW = kColumns * kTileW + (kColumns - 1) * spacing + sb + 8;
    resize(gridW, 818);
    setFixedWidth(gridW);

    QMenu *file = menuBar()->addMenu("&File");
    QAction *newWin = file->addAction("&New Window", [this]() {
        QProcess::startDetached(QCoreApplication::applicationFilePath());
    });
    newWin->setShortcut(QKeySequence::New);
    file->addAction("&Settings…", this, &MainWindow::onSettings);
    file->addAction("&Reload", this, &MainWindow::onReload);
    file->addAction("&Open Data Folder", this, &MainWindow::onOpenDataFolder);
    file->addSeparator();
    file->addAction("E&xit", qApp, &QApplication::quit);
    QMenu *help = menuBar()->addMenu("&Help");
    help->addAction("Jocala Catalog &Help", this, &MainWindow::onHelp);
    help->addAction("View &Changelog", this, &MainWindow::onChangelog);
    help->addSeparator();
    help->addAction("&About Jocala Catalog", this, &MainWindow::onAbout);

    QToolBar *bar = addToolBar("Main");
    bar->setMovable(false);
    // Gap after the arrow is fixed-width so it does not eat the toolbar
    // slack; the gap before the glyph group expands to pin it right.
    auto addSpacer = [bar](int fixedWidth) {
        QWidget *s = new QWidget(bar);
        s->setSizePolicy(fixedWidth > 0 ? QSizePolicy::Fixed : QSizePolicy::Expanding,
                         QSizePolicy::Preferred);
        if (fixedWidth > 0)
            s->setFixedWidth(fixedWidth);
        bar->addWidget(s);
    };
    // Back arrow: resets the display to the library root. Always visible,
    // so its position never shifts; at the root it simply re-reads.
    m_backAct = bar->addAction("←", this, &MainWindow::onLibraryBack);
    m_backAct->setToolTip("Back to Library");
    addSpacer(12);
    m_browseBox = new QComboBox(this);
    m_browseBox->addItems({"Books", "Author", "Series", "Tags"});
    m_browseBox->setToolTip("Browse: Books, Author, Series, or Tags");
    connect(m_browseBox, QOverload<int>::of(&QComboBox::currentIndexChanged),
            this, &MainWindow::onBrowseChanged);
    bar->addWidget(m_browseBox);
    addSpacer(12);
    m_sortBox = new QComboBox(this);
    m_sortBox->setToolTip("Sort order");
    connect(m_sortBox, QOverload<int>::of(&QComboBox::currentIndexChanged),
            this, &MainWindow::onSortChanged);
    bar->addWidget(m_sortBox);
    bar->addSeparator();
    addSpacer(0);
    // Search and the view toggles, right-aligned as one group.
    QAction *searchAct = bar->addAction("🔍", this, &MainWindow::onSearch);
    searchAct->setToolTip("Search Books");
    m_gridAct = bar->addAction("⊞", this, [this]() { onViewMode(true); });
    m_gridAct->setCheckable(true);
    m_gridAct->setChecked(true);
    m_gridAct->setToolTip("Grid view");
    m_listAct = bar->addAction("☰", this, [this]() { onViewMode(false); });
    m_listAct->setCheckable(true);
    m_listAct->setToolTip("List view");
    // These are bare glyphs, so bump their point size to read at a
    // glance beside the toolbar's text actions. QToolButton paints with its
    // own font (QAction::setFont would not reach it), so set it on the
    // button the toolbar built for each action.
    QFont glyphFont = bar->font();
    glyphFont.setPointSize(glyphFont.pointSize() + 3);
    for (QAction *act : {m_backAct, searchAct, m_gridAct, m_listAct}) {
        if (QToolButton *btn = qobject_cast<QToolButton *>(bar->widgetForAction(act)))
            btn->setFont(glyphFont);
    }

    m_stack = new QStackedWidget(this);
    m_grid = new QListView(this);
    m_grid->setViewMode(QListView::IconMode);
    m_grid->setResizeMode(QListView::Adjust);
    m_grid->setMovement(QListView::Static);
    m_grid->setSpacing(spacing);
    m_grid->setGridSize(QSize(kTileW, kTileH));
    // Minimum two grid rows high, never less (window can't collapse shut).
    const int minGridH = 2 * kTileH + spacing;
    m_grid->setMinimumHeight(minGridH);
    m_stack->setMinimumHeight(minGridH);
    m_grid->setHorizontalScrollBarPolicy(Qt::ScrollBarAlwaysOff);
    m_grid->setUniformItemSizes(true);
    m_grid->setModel(m_store.model());
    m_grid->setItemDelegate(new TileDelegate(this));
    m_grid->setMouseTracking(true);
    m_grid->viewport()->setCursor(Qt::PointingHandCursor);
    m_grid->setContextMenuPolicy(Qt::CustomContextMenu);
    connect(m_grid, &QListView::clicked, this, &MainWindow::onTileActivated);
    connect(m_grid, &QListView::activated, this, &MainWindow::onTileActivated);
    connect(m_grid, &QListView::customContextMenuRequested, this, &MainWindow::showContextMenu);
    m_list = new QListView(this);
    m_list->setModel(m_store.model());
    m_list->setUniformItemSizes(true);
    m_list->setHorizontalScrollBarPolicy(Qt::ScrollBarAlwaysOff);
    m_list->setIconSize(QSize(54, 60));
    m_list->setItemDelegate(new ListDelegate(this));
    m_list->viewport()->setCursor(Qt::PointingHandCursor);
    m_list->setContextMenuPolicy(Qt::CustomContextMenu);
    connect(m_list, &QListView::clicked, this, &MainWindow::onTileActivated);
    connect(m_list, &QListView::activated, this, &MainWindow::onTileActivated);
    connect(m_list, &QListView::customContextMenuRequested, this, &MainWindow::showContextMenu);
    m_emptyPage = new QWidget(this);
    QVBoxLayout *empty = new QVBoxLayout(m_emptyPage);
    m_emptyTitle = new QLabel("No books", m_emptyPage);
    m_emptyTitle->setAlignment(Qt::AlignCenter);
    QLabel *emptyHint = new QLabel("Set the SMB server and Calibre path in Settings → SMB", m_emptyPage);
    emptyHint->setAlignment(Qt::AlignCenter);
    QPushButton *openSettings = new QPushButton("Open Settings", m_emptyPage);
    connect(openSettings, &QPushButton::clicked, this, &MainWindow::onSettings);
    empty->addStretch();
    empty->addWidget(m_emptyTitle);
    empty->addWidget(emptyHint);
    empty->addWidget(openSettings, 0, Qt::AlignCenter);
    empty->addStretch();
    m_loadingPage = new QWidget(this);
    QVBoxLayout *loading = new QVBoxLayout(m_loadingPage);
    QProgressBar *spin = new QProgressBar(m_loadingPage);
    spin->setRange(0, 0);
    spin->setFixedWidth(200);
    QLabel *loadingLabel = new QLabel("Loading…", m_loadingPage);
    loadingLabel->setAlignment(Qt::AlignCenter);
    loading->addStretch();
    loading->addWidget(spin, 0, Qt::AlignCenter);
    loading->addWidget(loadingLabel);
    loading->addStretch();
    m_stack->addWidget(m_grid);
    m_stack->addWidget(m_list);
    m_stack->addWidget(m_emptyPage);
    m_stack->addWidget(m_loadingPage);
    setCentralWidget(m_stack);

    m_statusLabel = new QLabel(this);
    m_statusLabel->setContentsMargins(12, 4, 12, 4);
    // Single-line, overwrite-only: a new message replaces the old one
    // and can never wrap or grow the status bar.
    m_statusLabel->setWordWrap(false);
    statusBar()->addPermanentWidget(m_statusLabel, 1);
    // Window floor follows the two-row stack minimum plus chrome
    // (menu/toolbar/status) so dragging shut stops at two rows.
    setMinimumHeight(minimumSizeHint().height());
    restoreGeometrySetting();
    m_geomTimer.setSingleShot(true);
    m_geomTimer.setInterval(1000);
    connect(&m_geomTimer, &QTimer::timeout, this, &MainWindow::saveGeometrySetting);

    connect(&m_store, &CatalogStore::countsChanged, this, &MainWindow::onCounts);
    connect(&m_store, &CatalogStore::statusChanged, this, &MainWindow::onStatus);
    connect(&m_store, &CatalogStore::loadingChanged, this, &MainWindow::onLoading);
    connect(&m_store, &CatalogStore::dbError, this, &MainWindow::onDbError);
    connect(&m_store, &CatalogStore::koboOutcome, this, &MainWindow::onKoboOutcome);
    connect(&m_store, &CatalogStore::handoffOffer, this, &MainWindow::onHandoffOffer);
    connect(&m_store, &CatalogStore::detailReady, this, &MainWindow::onDetailReady);

    restoreToolbar();
    m_store.reload();
    if (m_store.settings().checkForUpdates)
        m_updater.check(this, true); // startup: silent unless an update is ready
}

// Sort enum -> combo label. Kept by name, never by position: the combo
// order is a UI choice (Books leads with Newest/Oldest) and must not
// depend on the enum's declaration order.
static QString sortLabel(CatalogStore::Sort s) {
    switch (s) {
    case CatalogStore::Newest: return QStringLiteral("Newest");
    case CatalogStore::Oldest: return QStringLiteral("Oldest");
    case CatalogStore::AZ: return QStringLiteral("A–Z");
    case CatalogStore::ZA: return QStringLiteral("Z–A");
    case CatalogStore::ByAuthor: break;
    }
    return QStringLiteral("Author");
}

void MainWindow::refreshSortBox() {
    m_sortBox->blockSignals(true);
    m_sortBox->clear();
    // Drilled series grid: Index (= series_index) replaces Author.
    if (m_store.isSeriesDrill()) {
        m_sortBox->addItems({"Index", "A–Z", "Z–A"});
        CatalogStore::Sort s = m_store.sort();
        m_sortBox->setCurrentIndex(s == CatalogStore::ZA ? 2 : s == CatalogStore::AZ ? 1 : 0);
        m_sortBox->blockSignals(false);
        return;
    }
    switch (m_store.mode()) {
    case CatalogStore::Books:
        m_sortBox->addItems({"Newest", "Oldest", "Author", "A–Z", "Z–A"});
        break;
    case CatalogStore::Authors:
        m_sortBox->addItems({"A–Z", "Z–A"});
        break;
    case CatalogStore::Series:
        m_sortBox->addItems({"Author", "A–Z", "Z–A"});
        break;
    case CatalogStore::Tags:
        m_sortBox->addItems({"A–Z", "Z–A"});
        break;
    }
    // Reflect the sort actually in effect. A blind row 0 would now claim
    // "Newest" whenever the store is still on Author.
    const int cur = m_sortBox->findText(sortLabel(m_store.sort()));
    m_sortBox->setCurrentIndex(cur >= 0 ? cur : 0);
    m_sortBox->blockSignals(false);
}

void MainWindow::restoreToolbar() {
    // Saved toolbar state, applied before the single startup load.
    // Out-of-range values fall back to row 0 (fresh-start behavior).
    AppSettings s = m_store.settings();
    auto mode = (CatalogStore::Mode)qBound(0, s.browseMode, 3);
    auto sort = (CatalogStore::Sort)qBound(0, s.sortOrder, 4);
    m_store.setInitialView(mode, sort);
    m_browseBox->blockSignals(true);
    m_browseBox->setCurrentIndex((int)mode);
    m_browseBox->blockSignals(false);
    refreshSortBox();
    QString want = "A–Z";
    if (mode == CatalogStore::Books) {
        want = sortLabel(sort);
    } else if (mode == CatalogStore::Series) {
        want = sort == CatalogStore::ZA ? "Z–A" : sort == CatalogStore::AZ ? "A–Z" : "Author";
    } else if (sort == CatalogStore::ZA) {
        want = "Z–A";
    }
    m_sortBox->blockSignals(true);
    int idx = m_sortBox->findText(want);
    m_sortBox->setCurrentIndex(idx >= 0 ? idx : 0);
    m_sortBox->blockSignals(false);
    onViewMode(s.viewMode != "list");
}

void MainWindow::restoreGeometrySetting() {
    // Position + height persist across launches (width is fixed by the
    // shell). A blob from a disconnected monitor falls back to centered
    // on the primary screen instead of stranding the window off-screen.
    QByteArray g = m_store.settings().windowGeometry;
    if (g.isEmpty())
        return;
    restoreGeometry(g);
    if (!QGuiApplication::screenAt(frameGeometry().center())) {
        QRect avail = QGuiApplication::primaryScreen()->availableGeometry();
        move(avail.center().x() - width() / 2, avail.center().y() - height() / 2);
    }
}

void MainWindow::saveGeometrySetting() {
    // Route through the store so the in-memory settings stay in sync:
    // a later saveView (toolbar change) rewrites the file from them and
    // would otherwise drop the geometry.
    AppSettings s = m_store.settings();
    s.windowGeometry = saveGeometry();
    AppSettings::save(s);
    m_store.setSettings(s);
}

void MainWindow::closeEvent(QCloseEvent *event) {
    saveGeometrySetting();
    QMainWindow::closeEvent(event);
}

void MainWindow::moveEvent(QMoveEvent *event) {
    QMainWindow::moveEvent(event);
    m_geomTimer.start();
}

void MainWindow::resizeEvent(QResizeEvent *event) {
    QMainWindow::resizeEvent(event);
    m_geomTimer.start();
}

QString MainWindow::currentView() const {
    return m_gridAct->isChecked() ? "grid" : "list";
}

void MainWindow::onBrowseChanged(int i) {
    m_koboActive = false;
    m_store.setMode((CatalogStore::Mode)i);
    refreshSortBox();
    m_store.saveView(currentView());
}

void MainWindow::onSortChanged(int i) {
    // Map per visible sort list back to the store enum.
    CatalogStore::Mode m = m_store.mode();
    CatalogStore::Sort s = CatalogStore::ByAuthor;
    QString t = m_sortBox->itemText(i);
    if (t == "A–Z") s = CatalogStore::AZ;
    else if (t == "Z–A") s = CatalogStore::ZA;
    else if (t == "Newest") s = CatalogStore::Newest;
    else if (t == "Oldest") s = CatalogStore::Oldest;
    else s = CatalogStore::ByAuthor; // "Author" and drilled "Index" share the slot
    Q_UNUSED(m);
    m_store.setSort(s);
    m_store.saveView(currentView());
}

void MainWindow::onSearch() {
    SearchDialog dlg(m_store.settings(), this);
    if (dlg.exec() != QDialog::Accepted)
        return;
    m_store.runSearch(dlg.params());
}

void MainWindow::onReload() {
    m_koboActive = false;
    m_store.reload(true);
    refreshSortBox();
}

void MainWindow::onSettings() {
    SettingsDialog dlg(m_store.settings(), this);
    if (dlg.exec() != QDialog::Accepted)
        return;
    AppSettings s = dlg.settings();
    AppSettings::save(s);
    m_store.setSettings(s);
    applyTheme(s.theme);
    m_store.reload(true);
}

void MainWindow::onViewMode(bool grid) {
    m_gridAct->setChecked(grid);
    m_listAct->setChecked(!grid);
    m_stack->setCurrentWidget(grid ? (QWidget *)m_grid : (QWidget *)m_list);
    m_store.saveView(currentView());
}

void MainWindow::onLibraryBack() {
    m_koboActive = false;
    m_store.reload();
    refreshSortBox();
}

void MainWindow::onTileActivated(const QModelIndex &idx) {
    if (!idx.isValid())
        return;
    CatalogStore::Mode m = m_store.mode();
    qint64 id = m_store.model()->idAt(idx.row());
    QString title = m_store.model()->data(idx, BookModel::TitleRole).toString();
    if (!m_store.showingBooks()) {
        if (m_store.searchSeriesMode()) {
            m_store.drillSeries(id, title);
        } else if (m == CatalogStore::Authors) {
            m_store.drillAuthor(id, title);
        } else if (m == CatalogStore::Series) {
            m_store.drillSeries(id, title);
        } else if (m == CatalogStore::Tags) {
            m_store.drillTag(title);
        }
        refreshSortBox();
        return;
    }
    openDetail(idx.row());
}

void MainWindow::openDetail(int row) {
    if (m_detailLoading)
        return; // first click wins; a second row clicked mid-load is
    // dropped instead of mixing its book with the in-flight detail.
    m_pendingBook = m_store.model()->bookAt(row);
    m_pendingRow = row;
    m_detailLoading = true;
    m_store.requestDetail(m_pendingBook.id);
}

void MainWindow::onDetailReady(const DetailItem &detail) {
    if (!m_detailLoading)
        return;
    m_detailLoading = false;
    m_pendingDetail = detail;
    BookInfoDialog dlg(m_pendingBook, detail, &m_store, this);
    QVariant cv = m_store.model()->data(m_store.model()->index(m_pendingRow, 0),
                                        BookModel::CoverRole);
    QPixmap pm = cv.value<QPixmap>();
    if (!pm.isNull())
        dlg.setCover(pm);
    dlg.exec();
}

void MainWindow::openKoboFor(const BookItem &book) {
    m_store.openOnKobo(book.id, book.title, book.author);
}

void MainWindow::showContextMenu(const QPoint &pos) {
    QListView *view = qobject_cast<QListView *>(sender());
    if (!view)
        return;
    QModelIndex idx = view->indexAt(pos);
    if (!idx.isValid() || !m_store.showingBooks())
        return;
    BookItem book = m_store.model()->bookAt(idx.row());
    QMenu menu(this);
    QAction *details = menu.addAction("Show Details");
    QAction *kobo = menu.addAction("Open on Kobo");
    QAction *chosen = menu.exec(view->viewport()->mapToGlobal(pos));
    if (chosen == details)
        openDetail(idx.row());
    else if (chosen == kobo)
        openKoboFor(book);
}

void MainWindow::onCounts(const QString &text) {
    // Idle counts live here; Kobo activity overrides until the next reload.
    if (m_koboActive)
        return;
    m_statusLabel->setText(text);
    m_statusLabel->setStyleSheet("color: gray");
}

void MainWindow::onStatus(const QString &text, bool kobo, bool ok) {
    // Flatten newlines (engine messages can be multiline) so the label
    // stays one line; assignment overwrites any previous message.
    QString t = text;
    t.replace('\n', ' ');
    if (!kobo) {
        onCounts(t);
        return;
    }
    m_koboActive = !t.isEmpty();
    m_statusLabel->setText(t);
    m_statusLabel->setStyleSheet(ok ? "color: green" : "color: orange");
}

void MainWindow::onLoading(bool loading) {
    if (loading) {
        m_stack->setCurrentWidget(m_loadingPage);
        return;
    }
    if (m_store.model()->rowCount() == 0) {
        m_stack->setCurrentWidget(m_emptyPage);
    } else if (m_gridAct->isChecked()) {
        m_stack->setCurrentWidget(m_grid);
    } else {
        m_stack->setCurrentWidget(m_list);
    }
}

void MainWindow::onDbError(const QString &message) {
    if (!m_store.settings().hasSource())
        return; // fresh start: silent, the empty page guides to Settings
    if (m_dbErrorOpen)
        return; // FFI error + watchdog share one dialog, never stacked
    // Deferred: onLoaded emits this synchronously from the watcher's
    // finished slot. Showing a modal dialog inline nests a modal event
    // loop inside the load chain (beachball look + reentrant reload).
    // Queue it so the load fully unwinds first.
    QString msg = message;
    m_dbErrorOpen = true;
    QTimer::singleShot(0, this, [this, msg]() { showDbError(msg); });
}

void MainWindow::showDbError(const QString &message) {
    QMessageBox::StandardButton r = QMessageBox::warning(
        this, "Calibre Database",
        message + "\nDetails saved to errors.log in the data folder.\n\nOpen Settings?",
        QMessageBox::Yes | QMessageBox::No | QMessageBox::Cancel);
    m_dbErrorOpen = false;
    if (r == QMessageBox::Yes)
        onSettings();
    else if (r == QMessageBox::No)
        m_store.reload();
}

void MainWindow::onHandoffOffer(const QString &ip) {
    Q_UNUSED(ip);
    QMessageBox box(QMessageBox::Question, "KOReader stacking fix",
        "KOReader can pile up reader instances when opening book after book. "
        "Install a tiny, reversible fix on the Kobo so each new book replaces the last? "
        "Reading goes ahead either way.",
        QMessageBox::NoButton, this);
    box.addButton("Install", QMessageBox::AcceptRole);
    box.addButton("Not Now", QMessageBox::RejectRole);
    box.exec();
    m_store.answerHandoff(box.clickedButton() &&
        box.buttonRole(box.clickedButton()) == QMessageBox::AcceptRole);
}

void MainWindow::onKoboOutcome(const QJsonObject &o) {    QString status = o.value("status").toString();
    if (status == "opened")
        return;
    if (status == "missing") {
        double size = o.value("size_bytes").toDouble(-1);
        QString sizeText = size >= 0
            ? QString("Sync this %1 book to Kobo via WiFi?").arg(
                  size > 1048576 ? QString("%1 MB").arg(size / 1048576.0, 0, 'f', 1)
                                 : QString("%1 KB").arg(qMax<qint64>(1, (qint64)(size / 1024))))
            : "Sync this book to Kobo via WiFi? (size unknown)";
        QMessageBox::StandardButton r = QMessageBox::question(
            this, "Not on Kobo",
            o.value("message").toString() + "\n\n" + sizeText,
            QMessageBox::Ok | QMessageBox::Cancel);
        if (r == QMessageBox::Ok && !m_pendingBook.title.isEmpty())
            m_store.syncOnKobo(m_pendingBook.id, m_pendingBook.title);
        return;
    }
    QMessageBox::warning(this, "Kobo", o.value("message").toString());
}

void MainWindow::onHelp() {
    HelpDialog dlg(this);
    dlg.exec();
}

void MainWindow::onAbout() {
    AboutDialog dlg(this);
    dlg.exec();
}

void MainWindow::onChangelog() {
    QDesktopServices::openUrl(QUrl("https://www.jocala.com/catalog/changelog.txt"));
}

void MainWindow::onOpenDataFolder() {
    QString dir = QFileInfo(AppSettings::settingsPath()).absolutePath();
    QDir().mkpath(dir);
    if (!QDesktopServices::openUrl(QUrl::fromLocalFile(dir)))
        QMessageBox::warning(this, "Data Folder", "Could not open:\n" + dir);
}
