#include "bookinfodialog.h"
#include "catalogstore.h"
#include <QDialogButtonBox>
#include <QHBoxLayout>
#include <QJsonObject>
#include <QPixmap>
#include <QProgressBar>
#include <QPushButton>
#include <QScrollArea>
#include <QVBoxLayout>

BookInfoDialog::BookInfoDialog(const BookItem &book, const DetailItem &detail,
                               CatalogStore *store, QWidget *parent)
    : QDialog(parent), m_book(book), m_store(store) {
    setWindowTitle("Book Details");
    resize(620, 560);
    QVBoxLayout *outer = new QVBoxLayout(this);
    QScrollArea *scroll = new QScrollArea(this);
    scroll->setWidgetResizable(true);
    QWidget *body = new QWidget(this);
    QVBoxLayout *top = new QVBoxLayout(body);
    QHBoxLayout *head = new QHBoxLayout();
    m_coverLabel = new QLabel(body);
    m_coverLabel->setFixedSize(160, 240);
    m_coverLabel->setAlignment(Qt::AlignCenter);
    m_coverLabel->setText("No cover");
    head->addWidget(m_coverLabel);
    QVBoxLayout *fields = new QVBoxLayout();
    auto add = [&](const QString &t, int px, bool muted) {
        QLabel *l = new QLabel(t, body);
        QFont f = l->font();
        f.setPointSize(px);
        l->setFont(f);
        l->setWordWrap(true);
        if (muted)
            l->setStyleSheet("color: gray");
        fields->addWidget(l);
    };
    add(detail.title.isEmpty() ? book.title : detail.title, 14, false);
    add(detail.author.isEmpty() ? book.author : detail.author, 11, true);
    if (!detail.series.isEmpty())
        add(QString("%1 #%2").arg(detail.series).arg(detail.seriesIndex), 10, true);
    if (!detail.tags.isEmpty())
        add(detail.tags, 10, true);
    if (!detail.publisher.isEmpty())
        add(detail.publisher, 10, true);
    if (!detail.isbn.isEmpty())
        add(QString("ISBN %1").arg(detail.isbn), 10, true);
    fields->addStretch();
    head->addLayout(fields);
    top->addLayout(head);
    if (!detail.comments.isEmpty()) {
        QLabel *c = new QLabel(detail.comments, body);
        c->setWordWrap(true);
        c->setTextInteractionFlags(Qt::TextSelectableByMouse);
        top->addWidget(c);
    }
    scroll->setWidget(body);
    outer->addWidget(scroll);
    QHBoxLayout *foot = new QHBoxLayout();
    QPushButton *readBtn = new QPushButton("📖 Read on Kobo", this);
    readBtn->setDefault(true);
    readBtn->setToolTip("Opens this book on the Kobo via KOReader");
    connect(readBtn, &QPushButton::clicked, this, &BookInfoDialog::onRead);
    QPushButton *closeBtn = new QPushButton("Close", this);
    connect(closeBtn, &QPushButton::clicked, this, &QDialog::accept);
    m_statusLabel = new QLabel(this);
    m_statusLabel->setWordWrap(false);
    // Sync progress for this book: hidden unless its push is running.
    // Numbers ride the status label; the bar is deliberately textless.
    m_koboBar = new QProgressBar(this);
    m_koboBar->setFixedWidth(140);
    m_koboBar->setTextVisible(false);
    m_koboBar->setVisible(false);
    foot->addWidget(readBtn);
    foot->addWidget(closeBtn);
    foot->addWidget(m_statusLabel, 1);
    foot->addWidget(m_koboBar, 0);
    outer->addLayout(foot);
    connect(m_store, &CatalogStore::statusChanged, this, &BookInfoDialog::onKoboStatus);
    connect(m_store, &CatalogStore::koboProgress, this, &BookInfoDialog::onKoboProgress);
    connect(m_store, &CatalogStore::koboOutcome, this, &BookInfoDialog::onKoboOutcome);
}

void BookInfoDialog::onRead() {
    m_store->openOnKobo(m_book.id, m_book.title, m_book.author);
}

void BookInfoDialog::setCover(const QPixmap &pm) {
    if (!pm.isNull())
        m_coverLabel->setPixmap(
            pm.scaled(160, 240, Qt::KeepAspectRatio, Qt::SmoothTransformation));
}

void BookInfoDialog::onKoboStatus(const QString &text, bool kobo, bool ok) {
    if (!kobo)
        return;
    QString t = text;
    t.replace('\n', ' ');
    m_statusLabel->setText(t);
    m_statusLabel->setStyleSheet(ok ? "color: green" : "color: orange");
    if (ok)
        accept();
}

void BookInfoDialog::onKoboProgress(const QString &title, qint64 done, qint64 total) {
    if (!m_koboBar || title != m_book.title)
        return; // another book's push (dialogs share one store)
    if (total <= 0) {
        // Fetch phase (EPUB off SMB/local, no total yet): alive, busy bar.
        m_koboBar->setRange(0, 0);
        m_koboBar->setVisible(true);
        return;
    }
    // Permille range: book bytes overflow an int range, permille never does.
    m_koboBar->setRange(0, 1000);
    m_koboBar->setValue((int)(done * 1000 / qMax<qint64>(1, total)));
    m_koboBar->setVisible(true);
    if (done != m_koboLastDone) {
        m_koboLastDone = done;
        m_koboStall.restart();
    }
    auto mb = [](qint64 b) -> QString {
        return b > 1048576 ? QString("%1 MB").arg(b / 1048576.0, 0, 'f', 1)
                           : QString("%1 KB").arg(qMax<qint64>(1, b / 1024));
    };
    QString text = QString("Syncing — %1 of %2").arg(mb(done), mb(total));
    if (m_koboStall.isValid() && m_koboStall.elapsed() > 15000 && done < total)
        text += " (stalled — leave it or force-quit; retry resumes)";
    m_statusLabel->setText(text);
    m_statusLabel->setStyleSheet("color: orange");
}

void BookInfoDialog::onKoboOutcome(const QJsonObject &o) {
    Q_UNUSED(o);
    if (m_koboBar)
        m_koboBar->setVisible(false);
    m_koboLastDone = -1;
    m_koboStall.invalidate();
}
