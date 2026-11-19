#pragma once
// Blocking Kobo FFI calls (run off the UI thread). Returns outcome JSON.
#include "settings.h"
#include <QJsonObject>
#include <QString>

struct KoboJob {
    // Identify the book on the device; never opens anything.
    // {"status":"found","auto","author_matched","candidates":[{path,title,author,tier}]}
    // | {"status":"missing",...} | {"status":"failed",...}
    static QJsonObject resolve(const AppSettings &s, const QString &ip,
                               const QString &title, const QString &author,
                               qint64 bookId);
    // Open an explicit device path (validate + KOReader launch).
    static QJsonObject openPath(const AppSettings &s, const QString &ip,
                                const QString &path);
    static QJsonObject sync(const AppSettings &s, const QString &ip, qint64 bookId);
    // Sync progress readout: {"done","total"} decoded bytes (0,0 = fetching).
    static QJsonObject progress();
    static QJsonObject ssh(const QString &ip, const QString &cmd, const QString &pw,
                           int timeoutSecs);
    // Stacking-fix preflight over the russh transport (password included).
    // Returns {"installed",...} / {"state",...}, or {"status":"failed",...}.
    static QJsonObject handoffCheck(const QString &ip, const QString &pw);
    static QJsonObject handoffEnsure(const QString &ip, const QString &pw);
    static QString koboAuthPassword(const AppSettings &s, const QString &ip);
};
