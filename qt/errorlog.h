#pragma once
// User-accessible error log: one capped text file beside settings.json
// (File -> Open Data Folder reveals it). Support flow: the user attaches
// errors.log and every library outage is diagnosable from it alone —
// launch header (version/platform/source shape) plus one full-text line
// per failure. Call sites pass pre-formed display strings; this writer
// never sees config JSON or credential fields, so secrets can't leak.
#include <QMutex>
#include <QString>

class AppSettings;

class ErrorLog {
public:
    explicit ErrorLog(const QString &rootDir,
                      qint64 maxBytes = 256LL * 1024);
    static QString defaultRoot(); // dir holding settings.json
    static QString defaultFile() { return defaultRoot() + "/errors.log"; }

    // Single line: timestamp + [category] + message. Newlines flattened.
    void log(const QString &category, const QString &message);
    // Launch header: version/platform/source shape, no secrets.
    void logHeader(const QString &version, const AppSettings &settings);
    // Source shape for the header: host/share/path/user only. Never
    // passwords (they live in AppSettings::passwords, unread here).
    static QString sourceSummary(const AppSettings &settings);

private:
    void trimLocked();
    QString m_file;
    qint64 m_max;
    QMutex m_gate;
};
