// DiskCoverCache: round-trip, key stability, oldest-first eviction.
// QTemporaryDir only — never touches the real cache.
#include "../covercache.h"
#include <QTemporaryDir>
#include <QTest>
#include <QThread>

class TstCoverCache : public QObject {
    Q_OBJECT
public:
    QTemporaryDir m_tmp;
private slots:
    void initTestCase() { QVERIFY(m_tmp.isValid()); }
    void roundtrip() {
        DiskCoverCache c(m_tmp.path());
        QByteArray out;
        QVERIFY(!c.tryGet("smb://h/s/Author/Title (1)", out));
        c.put("smb://h/s/Author/Title (1)", QByteArray("jpeg-bytes"));
        QVERIFY(c.tryGet("smb://h/s/Author/Title (1)", out));
        QCOMPARE(out, QByteArray("jpeg-bytes"));
    }
    void keyStable() {
        QCOMPARE(DiskCoverCache::keyFor("a"), DiskCoverCache::keyFor("a"));
        QVERIFY(DiskCoverCache::keyFor("a") != DiskCoverCache::keyFor("b"));
        QVERIFY(DiskCoverCache::keyFor("a").endsWith(".jpg"));
    }
    void evictsOldestFirst() {
        DiskCoverCache c(m_tmp.path() + "/cap", 10);
        c.put("a", QByteArray(6, 'x'));
        QThread::msleep(20);
        c.put("b", QByteArray(6, 'y'));
        QByteArray out;
        QVERIFY(!c.tryGet("a", out));
        QVERIFY(c.tryGet("b", out));
    }
};

QTEST_MAIN(TstCoverCache)
#include "tst_covercache.moc"
