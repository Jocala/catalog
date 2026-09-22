#include "settings.h"
#include <QDir>
#include <QFile>
#include <QJsonArray>
#include <QJsonDocument>
#include <QStandardPaths>

QString AppSettings::settingsPath() {
    // Windows always sets APPDATA (shared %APPDATA%/com.jocala.Catalog
    // file with the WPF app). Elsewhere (Linux) it is empty, which used
    // to build the root-anchored garbage "/com.jocala.Catalog/..." —
    // unwritable, so settings silently never persisted. Fall back to
    // the platform app-data location instead.
    QString appdata = qEnvironmentVariable("APPDATA");
    QString base = appdata.isEmpty()
        ? QStandardPaths::writableLocation(QStandardPaths::AppDataLocation)
        : appdata + "/com.jocala.Catalog";
    return base + "/settings.json";
}

QString AppSettings::smbPassword(const QString &host) const {
    QString p = passwords.value(host).toString();
    return p.isEmpty() ? QString() : p;
}

void AppSettings::setSmbPassword(const QString &host, const QString &pw) {
    if (pw.isEmpty())
        passwords.remove(host);
    else
        passwords.insert(host, pw);
}

QString AppSettings::koboPassword(const QString &ip) const {
    QString p = koboPasswords.value(ip.trimmed()).toString();
    return p.isEmpty() ? QString() : p;
}

void AppSettings::setKoboPassword(const QString &ip, const QString &pw) {
    QString t = ip.trimmed();
    if (t.isEmpty())
        return;
    if (pw.isEmpty())
        koboPasswords.remove(t);
    else
        koboPasswords.insert(t, pw);
}

void AppSettings::renameKobo(const QString &oldIp, const QString &newIp) {
    QString o = oldIp.trimmed(), n = newIp.trimmed();
    if (o.isEmpty() || n.isEmpty() || o == n)
        return;
    int idx = koboIps.indexOf(o);
    if (idx < 0)
        return;
    koboIps[idx] = n;
    if (koboPasswords.contains(o)) {
        QString pw = koboPasswords.value(o).toString();
        koboPasswords.remove(o);
        if (!pw.isEmpty())
            koboPasswords.insert(n, pw);
    }
    if (koboIp == o)
        koboIp = n;
}

const SmbServer *AppSettings::primaryServer() const {
    return servers.isEmpty() ? nullptr : &servers.first();
}

bool AppSettings::hasSource() const {
    if (librarySource == "local")
        return !localDir.trimmed().isEmpty();
    return !servers.isEmpty() && !servers.first().host.trimmed().isEmpty();
}

QString AppSettings::libraryConfigJson(bool fresh) const {
    QJsonObject o;
    if (fresh)
        o.insert("fresh", "1");
    if (librarySource == "local") {
        o.insert("source", "local");
        o.insert("local_dir", localDir);
        return QString::fromUtf8(QJsonDocument(o).toJson(QJsonDocument::Compact));
    }
    o.insert("source", "smb");
    o.insert("local_dir", "");
    const SmbServer *s = primaryServer();
    o.insert("host", s ? s->host : "");
    QString share, remote, user, domain, pass;
    if (s && !s->shares.isEmpty()) {
        share = s->shares.first().name;
        remote = s->shares.first().calibrePath;
        user = s->user;
        domain = s->domain;
        pass = smbPassword(s->host);
    }
    o.insert("share", share);
    o.insert("remote_dir", remote);
    o.insert("user", user);
    o.insert("pass", pass);
    o.insert("domain", domain);
    return QString::fromUtf8(QJsonDocument(o).toJson(QJsonDocument::Compact));
}

AppSettings AppSettings::load() {
    AppSettings s;
    QFile f(settingsPath());
    if (!f.open(QIODevice::ReadOnly))
        return s;
    QJsonDocument doc = QJsonDocument::fromJson(f.readAll());
    if (!doc.isObject())
        return s;
    QJsonObject o = doc.object();
    s.librarySource = o.value("LibrarySource").toString("smb");
    s.localDir = o.value("LocalLibraryDir").toString();
    for (const QJsonValue &sv : o.value("SmbServers").toArray()) {
        QJsonObject so = sv.toObject();
        SmbServer srv;
        srv.label = so.value("Label").toString();
        srv.host = so.value("Host").toString();
        srv.port = so.value("Port").toInt(445);
        srv.user = so.value("User").toString();
        srv.domain = so.value("Domain").toString();
        for (const QJsonValue &shv : so.value("Shares").toArray()) {
            QJsonObject sho = shv.toObject();
            SmbShare sh;
            sh.name = sho.value("Name").toString();
            sh.calibrePath = sho.value("CalibreMetadataPath").toString();
            srv.shares.append(sh);
        }
        s.servers.append(srv);
    }
    s.passwords = o.value("Passwords").toObject();
    s.koboIp = o.value("KoboIp").toString();
    for (const QJsonValue &kv : o.value("KoboIps").toArray())
        s.koboIps.append(kv.toString());
    s.koboPasswords = o.value("KoboPasswords").toObject();
    s.theme = o.value("ThemePreference").toInt(0);
    return s;
}

bool AppSettings::save(const AppSettings &s) {
    QJsonObject o;
    o.insert("LibrarySource", s.librarySource);
    o.insert("LocalLibraryDir", s.localDir);
    QJsonArray servers;
    for (const SmbServer &srv : s.servers) {
        QJsonObject so;
        so.insert("Label", srv.label);
        so.insert("Host", srv.host);
        so.insert("Port", srv.port);
        so.insert("User", srv.user);
        so.insert("Domain", srv.domain);
        QJsonArray shares;
        for (const SmbShare &sh : srv.shares) {
            QJsonObject sho;
            sho.insert("Name", sh.name);
            sho.insert("CalibreMetadataPath", sh.calibrePath);
            shares.append(sho);
        }
        so.insert("Shares", shares);
        servers.append(so);
    }
    o.insert("SmbServers", servers);
    o.insert("Passwords", s.passwords);
    o.insert("KoboIp", s.koboIp);
    QJsonArray ips;
    for (const QString &ip : s.koboIps)
        ips.append(ip);
    o.insert("KoboIps", ips);
    o.insert("KoboPasswords", s.koboPasswords);
    o.insert("ThemePreference", s.theme);
    QFile f(settingsPath());
    QDir().mkpath(QFileInfo(f).absolutePath());
    if (!f.open(QIODevice::WriteOnly | QIODevice::Truncate))
        return false;
    f.write(QJsonDocument(o).toJson(QJsonDocument::Indented));
    return true;
}
