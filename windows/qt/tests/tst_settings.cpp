// Settings JSON round-trips the exact WPF schema keys.
// APPDATA is redirected to a temp dir: never touch real settings.
#include "../settings.h"
#include <QJsonDocument>
#include <QTemporaryDir>
#include <QTest>

class TstSettings : public QObject {
    Q_OBJECT
public:
    QTemporaryDir m_tmp;
private slots:
    void initTestCase() {
        QVERIFY(m_tmp.isValid());
        qputenv("APPDATA", m_tmp.path().toUtf8());
    }
    void roundtrip() {
        AppSettings s;
        s.librarySource = "smb";
        SmbServer srv;
        srv.label = "nas";
        srv.host = "192.168.1.39";
        srv.port = 445;
        srv.user = "jeff";
        srv.domain = "";
        SmbShare sh;
        sh.name = "ebooks";
        sh.calibrePath = "calibre/";
        srv.shares.append(sh);
        s.servers.append(srv);
        s.setSmbPassword("192.168.1.39", "secret");
        s.koboIp = "192.168.1.74";
        s.koboIps = {"192.168.1.74", "192.168.1.75"};
        s.setKoboPassword("192.168.1.74", "1234");
        s.theme = 2;
        QVERIFY(AppSettings::save(s));
        AppSettings back = AppSettings::load();
        QCOMPARE(back.librarySource, QString("smb"));
        QCOMPARE(back.servers.size(), 1);
        QCOMPARE(back.servers.first().host, QString("192.168.1.39"));
        QCOMPARE(back.servers.first().shares.first().calibrePath, QString("calibre/"));
        QCOMPARE(back.smbPassword("192.168.1.39"), QString("secret"));
        QCOMPARE(back.koboIp, QString("192.168.1.74"));
        QCOMPARE(back.koboIps.size(), 2);
        QCOMPARE(back.koboPassword("192.168.1.74"), QString("1234"));
        QVERIFY(back.koboPassword("10.0.0.9").isNull());
        QCOMPARE(back.theme, 2);
        back.renameKobo("192.168.1.74", "192.168.1.76");
        QCOMPARE(back.koboIp, QString("192.168.1.76"));
        QCOMPARE(back.koboPassword("192.168.1.76"), QString("1234"));
        QVERIFY(back.koboPassword("192.168.1.74").isNull());
    }

    void libraryConfig() {
        AppSettings s;
        SmbServer srv;
        srv.host = "h";
        srv.user = "u";
        SmbShare sh;
        sh.name = "share";
        sh.calibrePath = "calibre/";
        srv.shares.append(sh);
        s.servers.append(srv);
        s.setSmbPassword("h", "p");
        QJsonDocument doc = QJsonDocument::fromJson(s.libraryConfigJson().toUtf8());
        QVERIFY(doc.isObject());
        QJsonObject o = doc.object();
        QCOMPARE(o.value("source").toString(), QString("smb"));
        QCOMPARE(o.value("host").toString(), QString("h"));
        QCOMPARE(o.value("share").toString(), QString("share"));
        QCOMPARE(o.value("remote_dir").toString(), QString("calibre/"));
        QCOMPARE(o.value("user").toString(), QString("u"));
        QCOMPARE(o.value("pass").toString(), QString("p"));
    }
};

QTEST_MAIN(TstSettings)
#include "tst_settings.moc"
