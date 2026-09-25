#include "errorlog.h"
#include "settings.h"
#include <QDateTime>
#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QSysInfo>

ErrorLog::ErrorLog(const QString &rootDir, qint64 maxBytes)
    : m_file(rootDir + "/errors.log"), m_max(maxBytes) {
    QDir().mkpath(rootDir);
}

QString ErrorLog::defaultRoot() {
    return QFileInfo(AppSettings::settingsPath()).absolutePath();
}

static QString flat(QString s) {
    s.replace('\n', ' ');
    s.replace('\r', ' ');
    return s.trimmed();
}

void ErrorLog::log(const QString &category, const QString &message) {
    QMutexLocker lock(&m_gate);
    QFile f(m_file);
    if (!f.open(QIODevice::Append | QIODevice::Text))
        return;
    QString line = QString("%1 [%2] %3\n")
                       .arg(QDateTime::currentDateTime().toString(Qt::ISODateWithMs),
                            flat(category), flat(message));
    f.write(line.toUtf8());
    f.close();
    trimLocked();
}

void ErrorLog::logHeader(const QString &version, const AppSettings &settings) {
    log("startup",
        QString("Catalog %1 Qt %2 %3 %4 source %5")
            .arg(version, qVersion(), QSysInfo::prettyProductName(),
                 QSysInfo::currentCpuArchitecture(), sourceSummary(settings)));
}

QString ErrorLog::sourceSummary(const AppSettings &settings) {
    if (settings.librarySource == "local")
        return "local dir=" + settings.localDir;
    const SmbServer *s = settings.primaryServer();
    if (!s || s->host.trimmed().isEmpty())
        return "smb (unconfigured)";
    QString share = s->shares.isEmpty() ? QString() : s->shares.first().name;
    QString path = s->shares.isEmpty() ? QString() : s->shares.first().calibrePath;
    // Host/share/path/user only: passwords stay in settings, never here.
    return QString("smb host=%1 share=%2 path=%3 user=%4")
        .arg(s->host, share, path, s->user);
}

void ErrorLog::trimLocked() {
    QFile f(m_file);
    if (!f.open(QIODevice::ReadOnly))
        return;
    if (f.size() <= m_max)
        return;
    QByteArray all = f.readAll();
    f.close();
    // Keep the tail: drop whole lines from the front until under cap.
    int cut = all.size() - (int)(m_max * 3 / 4);
    int nl = all.indexOf('\n', cut);
    QByteArray tail = (nl >= 0) ? all.mid(nl + 1) : all.right((int)(m_max * 3 / 4));
    if (f.open(QIODevice::WriteOnly | QIODevice::Truncate))
        f.write(tail);
}
