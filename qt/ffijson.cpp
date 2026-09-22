#include "ffijson.h"
#include "ffi.h"
#include <QJsonDocument>
#include <QJsonObject>

bool ffiOk(char *raw, QJsonValue &payload, QString &err) {
    if (!raw) {
        err = "ffi returned null";
        return false;
    }
    QByteArray buf(raw);
    catalog_string_free(raw);
    QJsonParseError perr;
    QJsonDocument doc = QJsonDocument::fromJson(buf, &perr);
    if (perr.error != QJsonParseError::NoError) {
        err = "bad ffi json: " + perr.errorString();
        return false;
    }
    if (!doc.isObject()) {
        err = "bad ffi envelope";
        return false;
    }
    QJsonObject o = doc.object();
    if (o.contains("ok")) {
        payload = o.value("ok");
        return true;
    }
    err = o.value("error").toString("unknown ffi error");
    return false;
}

QByteArray ffiBytes(struct CatalogBytes b) {
    QByteArray out;
    if (b.ptr && b.len > 0)
        out = QByteArray(b.ptr, (qsizetype)b.len);
    catalog_bytes_free(b);
    return out;
}
