// Settings JSON round-trips the exact WPF schema keys.
// APPDATA is redirected to a temp dir: never touch real settings.
#include "../settings.h"
#include "../theme.h"
#include <QApplication>
#include <QDir>
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
        s.browseMode = 2;
        s.sortOrder = 3;
        s.viewMode = "list";
        QVERIFY(AppSettings::save(s));
        AppSettings back = AppSettings::load();
        QCOMPARE(back.librarySource, QString("smb"));
        QCOMPARE(back.servers.size(), 1);
        QCOMPARE(back.servers.first().host, QString("192.168.1.39"));
        QCOMPARE(back.servers.first().shares.first().calibrePath, QString("calibre/"));
        QCOMPARE(back.smbPassword("192.168.1.39"), QString("secret"));
        QCOMPARE(back.koboIp, QString("192.168.1.74"));
        QCOMPARE(back.koboIps.size(), 2);
        QVERIFY(!back.koboHandoffPromptDone);
        QCOMPARE(back.koboPassword("192.168.1.74"), QString("1234"));
        QVERIFY(back.koboPassword("10.0.0.9").isNull());
        QCOMPARE(back.theme, 2);
        QCOMPARE(back.browseMode, 2);
        QCOMPARE(back.sortOrder, 3);
        QCOMPARE(back.viewMode, QString("list"));
        // Update opt-out persists; fresh installs default to checked.
        QVERIFY(AppSettings().checkForUpdates);
        s.checkForUpdates = false;
        QVERIFY(AppSettings::save(s));
        QVERIFY(!AppSettings::load().checkForUpdates);
        // Diagnostic logging persists; fresh installs default to off.
        QVERIFY(!AppSettings().diagLogging);
        s.diagLogging = true;
        QVERIFY(AppSettings::save(s));
        QVERIFY(AppSettings::load().diagLogging);
        // Window geometry round-trips (empty by default, opaque blob).
        QVERIFY(AppSettings().windowGeometry.isEmpty());
        s.windowGeometry = QByteArray("geom-blob");
        QVERIFY(AppSettings::save(s));
        QCOMPARE(AppSettings::load().windowGeometry, QByteArray("geom-blob"));
        // Out-of-range view state clamps back to fresh-start defaults.
        back.browseMode = 9;
        back.sortOrder = 9;
        back.viewMode = "bogus";
        QVERIFY(AppSettings::save(back));
        AppSettings clamped = AppSettings::load();
        QCOMPARE(clamped.browseMode, 3);
        QCOMPARE(clamped.sortOrder, 4);
        QCOMPARE(clamped.viewMode, QString("grid"));
        back.koboHandoffPromptDone = true;
        QVERIFY(AppSettings::save(back));
        QVERIFY(AppSettings::load().koboHandoffPromptDone);
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
        // Ephemeral flags default off: no fresh/diag keys unless asked
        // (and the FFI strips them from its cache key when present).
        QVERIFY(!o.contains("fresh"));
        QVERIFY(!o.contains("diag"));
        QJsonObject fresh = QJsonDocument::fromJson(
            s.libraryConfigJson(true, true).toUtf8()).object();
        QCOMPARE(fresh.value("fresh").toString(), QString("1"));
        QCOMPARE(fresh.value("diag").toString(), QString("1"));
        QCOMPARE(fresh.value("host").toString(), QString("h"));
    }

    void hasSource() {
        AppSettings fresh;
        QVERIFY(!fresh.hasSource());
        AppSettings smb;
        SmbServer srv;
        srv.host = "h";
        smb.servers.append(srv);
        QVERIFY(smb.hasSource());
        AppSettings local;
        local.librarySource = "local";
        QVERIFY(!local.hasSource());
        local.localDir = "D:/books";
        QVERIFY(local.hasSource());
    }

    void settingsPathWithoutAppdata() {
        // Linux: no APPDATA in env. Must fall back to a writable
        // platform location, never root-anchored garbage (lost settings).
        QByteArray saved = qgetenv("APPDATA");
        qunsetenv("APPDATA");
        QString p = AppSettings::settingsPath();
        if (!saved.isNull())
            qputenv("APPDATA", saved);
        QVERIFY(!p.startsWith("/com.jocala.Catalog"));
        QVERIFY(QDir(p).isAbsolute());
        QVERIFY(p.endsWith("/settings.json"));
    }

    void darkPalettePaintsPlaceholders() {
        // Regression: the hand-built dark palette once omitted
        // PlaceholderText, so every QLineEdit hint (Kobo IP, SMB
        // fields, …) painted nothing in dark mode.
        applyTheme(2);
        QVERIFY(QApplication::palette().color(QPalette::PlaceholderText).isValid());
        applyTheme(1);
    }
};

QTEST_MAIN(TstSettings)
#include "tst_settings.moc"
