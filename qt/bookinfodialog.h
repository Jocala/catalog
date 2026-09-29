#pragma once
#include "models.h"
#include <QDialog>
#include <QElapsedTimer>
#include <QLabel>
#include <QPixmap>

class CatalogStore;
class QProgressBar;

class BookInfoDialog : public QDialog {
    Q_OBJECT
public:
    BookInfoDialog(const BookItem &book, const DetailItem &detail,
                   CatalogStore *store, QWidget *parent = nullptr);
    void setCover(const QPixmap &pm);

private slots:
    void onRead();
    void onKoboStatus(const QString &text, bool kobo, bool ok);
    void onKoboProgress(const QString &title, qint64 done, qint64 total);
    void onKoboOutcome(const QJsonObject &outcome);

private:
    BookItem m_book;
    CatalogStore *m_store;
    QLabel *m_coverLabel;
    QLabel *m_statusLabel;
    QProgressBar *m_koboBar = nullptr; // sync progress (this book only)
    QElapsedTimer m_koboStall; // no-advance watch: frozen bar names the stall
    qint64 m_koboLastDone = -1;
};
