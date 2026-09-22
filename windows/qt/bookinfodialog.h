#pragma once
#include "models.h"
#include <QDialog>
#include <QLabel>
#include <QPixmap>

class CatalogStore;

class BookInfoDialog : public QDialog {
    Q_OBJECT
public:
    BookInfoDialog(const BookItem &book, const DetailItem &detail,
                   CatalogStore *store, QWidget *parent = nullptr);
    void setCover(const QPixmap &pm);

private slots:
    void onRead();
    void onKoboStatus(const QString &text, bool kobo, bool ok);

private:
    BookItem m_book;
    CatalogStore *m_store;
    QLabel *m_coverLabel;
    QLabel *m_statusLabel;
};
