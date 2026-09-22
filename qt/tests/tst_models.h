#pragma once
// Model parsing matches the FFI serde keys verbatim.
#include "../models.h"
#include <QJsonArray>
#include <QJsonDocument>
#include <QObject>
#include <QTest>

class TstModels : public QObject {
    Q_OBJECT
private slots:
    void books();
    void summaries();
    void detail();
};
