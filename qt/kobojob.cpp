#include "kobojob.h"
#include "ffi.h"
#include "ffijson.h"
#include <QJsonDocument>
#include <QJsonObject>

QString KoboJob::koboAuthPassword(const AppSettings &s, const QString &ip) {
    return s.koboPassword(ip);
}

static QJsonObject outcomeOf(char *raw) {
    QJsonValue payload;
    QString err;
    QJsonObject out;
    if (ffiOk(raw, payload, err) && payload.isObject())
        return payload.toObject();
    out.insert("status", "failed");
    out.insert("message", err);
    out.insert("code", -1);
    return out;
}

QJsonObject KoboJob::open(const AppSettings &s, const QString &ip,
                          const QString &title, const QString &author,
                          qint64 bookId) {
    QJsonObject cfg;
    cfg.insert("ip", ip);
    cfg.insert("password", s.koboPassword(ip));
    cfg.insert("title", title);
    cfg.insert("author", author);
    cfg.insert("book_id", (double)bookId);
    QJsonObject lib = QJsonDocument::fromJson(s.libraryConfigJson().toUtf8()).object();
    for (auto it = lib.begin(); it != lib.end(); ++it)
        cfg.insert(it.key(), it.value());
    QByteArray raw = QJsonDocument(cfg).toJson(QJsonDocument::Compact);
    return outcomeOf(catalog_kobo_open(raw.constData()));
}

QJsonObject KoboJob::sync(const AppSettings &s, const QString &ip, qint64 bookId) {
    QJsonObject cfg;
    cfg.insert("ip", ip);
    cfg.insert("password", s.koboPassword(ip));
    cfg.insert("book_id", (double)bookId);
    QJsonObject lib = QJsonDocument::fromJson(s.libraryConfigJson().toUtf8()).object();
    for (auto it = lib.begin(); it != lib.end(); ++it)
        cfg.insert(it.key(), it.value());
    QByteArray raw = QJsonDocument(cfg).toJson(QJsonDocument::Compact);
    return outcomeOf(catalog_kobo_sync(raw.constData()));
}

QJsonObject KoboJob::ssh(const QString &ip, const QString &cmd, const QString &pw,
                         int timeoutSecs) {
    QJsonObject cfg;
    cfg.insert("ip", ip);
    cfg.insert("cmd", cmd);
    cfg.insert("password", pw);
    cfg.insert("timeout_secs", timeoutSecs);
    QByteArray raw = QJsonDocument(cfg).toJson(QJsonDocument::Compact);
    return outcomeOf(catalog_kobo_ssh(raw.constData()));
}

QJsonObject KoboJob::handoffCheck(const QString &ip) {
    QByteArray ipB = ip.toUtf8();
    return outcomeOf(catalog_kobo_handoff_check(ipB.constData()));
}

QJsonObject KoboJob::handoffEnsure(const QString &ip) {
    QByteArray ipB = ip.toUtf8();
    return outcomeOf(catalog_kobo_handoff_ensure(ipB.constData()));
}
