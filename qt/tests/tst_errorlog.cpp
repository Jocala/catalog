// ErrorLog: append/trim, source summary without secrets, line shape.
// QTemporaryDir only — never touches the real data folder.
#include "../errorlog.h"
#include "../settings.h"
#include <QFile>
#include <QTemporaryDir>
#include <QTest>

class TstErrorLog : public QObject {
    Q_OBJECT
public:
    QTemporaryDir m_tmp;
private slots:
    void initTestCase() { QVERIFY(m_tmp.isValid()); }
    void appendsLine() {
        ErrorLog log(m_tmp.path());
        log.log("library", "kind=root gen=1 ms=30007 :: boom");
        QFile f(m_tmp.path() + "/errors.log");
        QVERIFY(f.open(QIODevice::ReadOnly | QIODevice::Text));
        QString line = QString::fromUtf8(f.readAll());
        QVERIFY(line.contains("[library]"));
        QVERIFY(line.contains("kind=root gen=1 ms=30007 :: boom"));
    }
    void flattensNewlines() {
        ErrorLog log(m_tmp.path() + "/flat");
        log.log("library", "a\nb");
        QFile f(m_tmp.path() + "/flat/errors.log");
        QVERIFY(f.open(QIODevice::ReadOnly | QIODevice::Text));
        QCOMPARE(QString::fromUtf8(f.readAll()).count('\n'), 1);
    }
    void trimsAtCap() {
        ErrorLog log(m_tmp.path() + "/cap", 200);
        for (int i = 0; i < 20; ++i)
            log.log("library", QString("line-%1 padding-padding-padding").arg(i));
        QFile f(m_tmp.path() + "/cap/errors.log");
        QVERIFY(f.open(QIODevice::ReadOnly));
        QVERIFY(f.size() <= 200);
        QString tail = QString::fromUtf8(f.readAll());
        QVERIFY(tail.contains("line-19")); // newest survives
        QVERIFY(!tail.contains("line-0")); // oldest trimmed
    }
    void summaryHasNoSecrets() {
        AppSettings s;
        s.librarySource = "smb";
        SmbServer srv;
        srv.host = "nas";
        srv.user = "jeff";
        SmbShare sh;
        sh.name = "ebooks";
        sh.calibrePath = "calibre";
        srv.shares.append(sh);
        s.servers.append(srv);
        s.setSmbPassword("nas", "s3cret");
        QString sum = ErrorLog::sourceSummary(s);
        QVERIFY(sum.contains("nas"));
        QVERIFY(sum.contains("ebooks"));
        QVERIFY(!sum.contains("s3cret"));
        QVERIFY(!sum.contains("Passwords"));
    }
    void headerHasNoSecrets() {
        AppSettings s;
        s.setSmbPassword("nas", "s3cret");
        ErrorLog log(m_tmp.path() + "/hdr");
        log.logHeader("9.9", s);
        QFile f(m_tmp.path() + "/hdr/errors.log");
        QVERIFY(f.open(QIODevice::ReadOnly | QIODevice::Text));
        QString line = QString::fromUtf8(f.readAll());
        QVERIFY(line.contains("[startup]"));
        QVERIFY(line.contains("9.9"));
        QVERIFY(!line.contains("s3cret"));
    }
};

QTEST_MAIN(TstErrorLog)
#include "tst_errorlog.moc"
