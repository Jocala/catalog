#pragma once

// Shared tag artwork for tag browse tiles, loaded from the Qt resource
// system so it is available in packaged builds as well as development.

#include <QRectF>

class QPainter;

void paintTagIcon(QPainter *painter, const QRectF &rect);
