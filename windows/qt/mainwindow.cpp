#include "mainwindow.h"
#include "aboutdialog.h"
#include "bookinfodialog.h"
#include "helpdialog.h"
#include "searchdialog.h"
#include "settingsdialog.h"
#include "theme.h"
#include <QAction>
#include <QApplication>
#include <QCoreApplication>
#include <QDesktopServices>
#include <QDialog>
#include <QDir>
#include <QHBoxLayout>
#include <QMenu>
#include <QMenuBar>
#include <QMessageBox>
#include <QPainter>
#include <QProcess>
#include <QProgressBar>
#include <QPushButton>
#include <QStandardPaths>
#include <QStatusBar>
#include <QToolBar>
#include <QUrl>
#include <QVBoxLayout>

TileDelegate::TileDelegate(QObject *parent) : QStyledItemDelegate(parent) {}

QSize TileDelegate::sizeHint(const QStyleOptionViewItem &, const QModelIndex &) const {
    return QSize(200, 300);
}

void TileDelegate::paint(QPainter *p, const QStyleOptionViewItem &opt,
                         const QModelIndex &idx) const {
    p->save();
    if (opt.state & QStyle::State_Selected) {
        p->fillRect(opt.rect, opt.palette.highlight());
    } else if (opt.state & QStyle::State_MouseOver) {
        p->fillRect(opt.rect, opt.palette.alternateBase());
    }
    QRect coverRect(opt.rect.x() + 6, opt.rect.y() + 4, 187, 240);
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
    QRect titleRect(opt.rect.x(), opt.rect.y() + 246, 200, 34);
    p->setPen(opt.palette.text().color());
    QFont f = p->font();
    f.setPointSize(9);
    p->setFont(f);
    p->drawText(titleRect, Qt::AlignHCenter | Qt::TextWordWrap,
                idx.data(BookModel::TitleRole).toString());
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
    resize(912, 818);

    QMenu *file = menuBar()->addMenu("&File");
    QAction *newWin = file->addAction("&New Window", [this]() {
        QProcess::startDetached(QCoreApplication::applicationFilePath());
    });
    newWin->setShortcut(QKeySequence::New);
    file->addAction("&Settings…", this, &MainWindow::onSettings);
    file->addAction("&Open Data Folder", this, &MainWindow::onOpenDataFolder);
    file->addSeparator();
    file->addAction("E&xit", qApp, &QApplication::quit);
    QMenu *help = menuBar()->addMenu("&Help");
    help->addAction("Jocala Catalog &Help", this, &MainWindow::onHelp);
    help->addSeparator();
    help->addAction("&About Jocala Catalog", this, &MainWindow::onAbout);

    QToolBar *bar = addToolBar("Main");
    bar->setMovable(false);
    QAction *searchAct = bar->addAction("🔍", this, &MainWindow::onSearch);
    searchAct->setToolTip("Search Books");
    bar->addAction("Reload", this, &MainWindow::onReload);
    bar->addAction("Settings", this, &MainWindow::onSettings);
    bar->addSeparator();
    m_browseBox = new QComboBox(this);
    m_browseBox->addItems({"Books", "Author", "Series", "Tags"});
    m_browseBox->setToolTip("Browse: Books, Author, Series, or Tags");
    connect(m_browseBox, QOverload<int>::of(&QComboBox::currentIndexChanged),
            this, &MainWindow::onBrowseChanged);
    bar->addWidget(m_browseBox);
    m_sortBox = new QComboBox(this);
    m_sortBox->setToolTip("Sort order");
    connect(m_sortBox, QOverload<int>::of(&QComboBox::currentIndexChanged),
            this, &MainWindow::onSortChanged);
    bar->addWidget(m_sortBox);
    bar->addSeparator();
    m_gridBtn = new QPushButton("Grid", this);
    m_gridBtn->setCheckable(true);
    m_gridBtn->setChecked(true);
    m_listBtn = new QPushButton("List", this);
    m_listBtn->setCheckable(true);
    connect(m_gridBtn, &QPushButton::clicked, this, [this]() { onViewMode(true); });
    connect(m_listBtn, &QPushButton::clicked, this, [this]() { onViewMode(false); });
    bar->addWidget(m_gridBtn);
    bar->addWidget(m_listBtn);
    m_libraryBtn = new QPushButton("Library", this);
    m_libraryBtn->setVisible(false);
    m_libraryBtn->setStyleSheet("font-weight: bold");
    connect(m_libraryBtn, &QPushButton::clicked, this, &MainWindow::onLibraryBack);
    bar->addWidget(m_libraryBtn);

    m_stack = new QStackedWidget(this);
    m_grid = new QListView(this);
    m_grid->setViewMode(QListView::IconMode);
    m_grid->setResizeMode(QListView::Adjust);
    m_grid->setMovement(QListView::Static);
    m_grid->setSpacing(12);
    m_grid->setUniformItemSizes(true);
    m_grid->setModel(m_store.model());
    m_grid->setItemDelegate(new TileDelegate(this));
    m_grid->setMouseTracking(true);
    m_grid->viewport()->setCursor(Qt::PointingHandCursor);
    m_grid->setContextMenuPolicy(Qt::CustomContextMenu);
    connect(m_grid, &QListView::activated, this, &MainWindow::onTileActivated);
    connect(m_grid, &QListView::customContextMenuRequested, this, &MainWindow::showContextMenu);
    m_list = new QListView(this);
    m_list->setModel(m_store.model());
    m_list->setUniformItemSizes(true);
    m_list->viewport()->setCursor(Qt::PointingHandCursor);
    m_list->setContextMenuPolicy(Qt::CustomContextMenu);
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
    statusBar()->addPermanentWidget(m_statusLabel, 1);

    connect(&m_store, &CatalogStore::countsChanged, this, &MainWindow::onCounts);
    connect(&m_store, &CatalogStore::statusChanged, this, &MainWindow::onStatus);
    connect(&m_store, &CatalogStore::loadingChanged, this, &MainWindow::onLoading);
    connect(&m_store, &CatalogStore::dbError, this, &MainWindow::onDbError);
    connect(&m_store, &CatalogStore::koboOutcome, this, &MainWindow::onKoboOutcome);
    connect(&m_store, &CatalogStore::detailReady, this, &MainWindow::onDetailReady);

    refreshSortBox();
    m_store.reload();
}

void MainWindow::refreshSortBox() {
    m_sortBox->blockSignals(true);
    m_sortBox->clear();
    switch (m_store.mode()) {
    case CatalogStore::Books:
        m_sortBox->addItems({"Author", "A–Z", "Z–A", "Newest", "Oldest"});
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
    m_sortBox->setCurrentIndex(0);
    m_sortBox->blockSignals(false);
}

void MainWindow::onBrowseChanged(int i) {
    m_koboActive = false;
    m_store.setMode((CatalogStore::Mode)i);
    refreshSortBox();
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
    else s = CatalogStore::ByAuthor;
    Q_UNUSED(m);
    m_store.setSort(s);
}

void MainWindow::onSearch() {
    SearchDialog dlg(m_store.settings(), this);
    if (dlg.exec() != QDialog::Accepted)
        return;
    m_store.runSearch(dlg.params());
    m_libraryBtn->setVisible(true);
}

void MainWindow::onReload() {
    m_libraryBtn->setVisible(false);
    m_koboActive = false;
    m_store.reload(true);
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
    m_gridBtn->setChecked(grid);
    m_listBtn->setChecked(!grid);
    m_stack->setCurrentWidget(grid ? (QWidget *)m_grid : (QWidget *)m_list);
}

void MainWindow::onLibraryBack() {
    m_libraryBtn->setVisible(false);
    m_libraryBtn->setText("Library");
    m_koboActive = false;
    m_store.reload();
}

void MainWindow::onTileActivated(const QModelIndex &idx) {
    if (!idx.isValid())
        return;
    CatalogStore::Mode m = m_store.mode();
    qint64 id = m_store.model()->idAt(idx.row());
    QString title = m_store.model()->data(idx, BookModel::TitleRole).toString();
    if (!m_store.showingBooks()) {
        m_libraryBtn->setText("‹ Back");
        m_libraryBtn->setVisible(true);
        if (m == CatalogStore::Authors) {
            m_store.drillAuthor(id, title);
        } else if (m == CatalogStore::Series) {
            m_store.drillSeries(id, title);
        } else if (m == CatalogStore::Tags) {
            m_store.drillTag(title);
        }
        return;
    }
    openDetail(idx.row());
}

void MainWindow::openDetail(int row) {
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
    if (!kobo) {
        onCounts(text);
        return;
    }
    m_koboActive = !text.isEmpty();
    m_statusLabel->setText(text);
    m_statusLabel->setStyleSheet(ok ? "color: green" : "color: orange");
}

void MainWindow::onLoading(bool loading) {
    if (loading) {
        m_stack->setCurrentWidget(m_loadingPage);
        return;
    }
    if (m_store.model()->rowCount() == 0) {
        m_stack->setCurrentWidget(m_emptyPage);
    } else if (m_gridBtn->isChecked()) {
        m_stack->setCurrentWidget(m_grid);
    } else {
        m_stack->setCurrentWidget(m_list);
    }
}

void MainWindow::onDbError(const QString &message) {
    if (!m_store.settings().hasSource())
        return; // fresh start: silent, the empty page guides to Settings
    QMessageBox::StandardButton r = QMessageBox::warning(
        this, "Calibre Database", message + "\n\nOpen Settings?",
        QMessageBox::Yes | QMessageBox::No | QMessageBox::Cancel);
    if (r == QMessageBox::Yes)
        onSettings();
    else if (r == QMessageBox::No)
        m_store.reload();
}

void MainWindow::onKoboOutcome(const QJsonObject &o) {
    QString status = o.value("status").toString();
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

void MainWindow::onOpenDataFolder() {
    QString dir = qEnvironmentVariable("APPDATA") + "/com.jocala.Catalog";
    QDesktopServices::openUrl(QUrl::fromLocalFile(dir));
}
