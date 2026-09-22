#pragma once
// Blocking Kobo FFI calls (run off the UI thread). Returns outcome JSON.
#include "settings.h"
#include <QJsonObject>
#include <QString>

struct KoboJob {
    // {"status","message","path","predicted","size_bytes","code","output"}
    static QJsonObject open(const AppSettings &s, const QString &ip,
                            const QString &title, const QString &author,
                            qint64 bookId);
    static QJsonObject sync(const AppSettings &s, const QString &ip, qint64 bookId);
    static QJsonObject ssh(const QString &ip, const QString &cmd, const QString &pw,
                           int timeoutSecs);
    static QString koboAuthPassword(const AppSettings &s, const QString &ip);
};
