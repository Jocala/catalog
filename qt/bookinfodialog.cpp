#include "bookinfodialog.h"
#include "catalogstore.h"
#include <QDialogButtonBox>
#include <QHBoxLayout>
#include <QPixmap>
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
    foot->addWidget(readBtn);
    foot->addWidget(closeBtn);
    foot->addWidget(m_statusLabel, 1);
    outer->addLayout(foot);
    connect(m_store, &CatalogStore::statusChanged, this, &BookInfoDialog::onKoboStatus);
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
