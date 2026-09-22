#pragma once
// FFI helpers: {"ok"} / {"error"} envelope parsing + RAII freeing.
#include <QByteArray>
#include <QJsonValue>
#include <QString>

struct CatalogBytes;
// Parses raw (frees it); true + payload on {"ok"}, else err text.
bool ffiOk(char *raw, QJsonValue &payload, QString &err);
// Copies bytes (frees the FFI allocation).
QByteArray ffiBytes(struct CatalogBytes b);
