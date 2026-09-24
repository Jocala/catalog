// BookModel cover roles: grid CoverRole serves the full pixmap, list
// DecorationRole serves a row-height thumbnail; both come from one batch
// insert, and a miss on either emits the shared coverNeeded demand.
// Images are generated in code — no fixtures.
#include "../bookmodel.h"
#include "../listdelegate.h"
#include <QImage>
#include <QPixmap>
#include <QSignalSpy>
#include <QTest>

class TstBookModel : public QObject {
    Q_OBJECT
private slots:
    void decorationRole() {
        BookModel m;
        BookItem b;
        b.id = 1;
        b.title = "T";
        b.author = "A";
        b.path = "p";
        m.setBooks({b});
        // Miss on both roles before any insert (and demand fires).
        QSignalSpy spy(&m, &BookModel::coverNeeded);
        QVERIFY(!m.data(m.index(0), BookModel::CoverRole).isValid());
        QVERIFY(!m.data(m.index(0), Qt::DecorationRole).isValid());
        QCOMPARE(spy.count(), 2);
        // One batch feeds both roles; decoration is row-height.
        QImage img(160, 220, QImage::Format_RGB32);
        img.fill(Qt::red);
        m.setCoverBatch({qMakePair(QString("p"), img)});
        QPixmap full = m.data(m.index(0), BookModel::CoverRole).value<QPixmap>();
        QVERIFY(!full.isNull());
        QCOMPARE(full.height(), 220);
        QPixmap small = m.data(m.index(0), Qt::DecorationRole).value<QPixmap>();
        QVERIFY(!small.isNull());
        QCOMPARE(small.height(), 180);
    }
    void listRowMetrics() {
        ListDelegate d;
        QStyleOptionViewItem opt;
        opt.rect = QRect(0, 0, 300, 196);
        QCOMPARE(d.sizeHint(opt, QModelIndex()).height(), ListDelegate::RowHeight);
    }
    void metaRoles() {
        BookModel m;
        BookItem b;
        b.id = 1;
        b.title = "Emma";
        b.author = "Jane Austen";
        b.path = "p";
        b.series = "Classics";
        b.seriesIndex = 1;
        b.tags = "Fiction, Romance";
        m.setBooks({b});
        QCOMPARE(m.data(m.index(0), BookModel::SeriesRole).toString(), QString("Classics"));
        QCOMPARE(m.data(m.index(0), BookModel::SeriesIndexRole).toDouble(), 1.0);
        QCOMPARE(m.data(m.index(0), BookModel::TagsRole).toString(),
                 QString("Fiction, Romance"));
    }
    void tagTileRole() {
        BookModel m;
        m.setTiles({"Fiction"}, {"1 books"}, {1}, {}, true);
        QVERIFY(m.data(m.index(0), BookModel::TagTileRole).toBool());

        BookItem b;
        b.id = 2;
        b.title = "Emma";
        m.setBooks({b});
        QVERIFY(!m.data(m.index(0), BookModel::TagTileRole).toBool());
    }
};

QTEST_MAIN(TstBookModel)
#include "tst_bookmodel.moc"
