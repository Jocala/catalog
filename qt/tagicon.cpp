#include "tagicon.h"

#include <QPainter>
#include <QPixmap>

namespace {
QPixmap loadTagImage() {
    QPixmap image;
    image.load(":/catalog/tag.png");
    return image;
}
}

void paintTagIcon(QPainter *painter, const QRectF &rect) {
    if (!painter || rect.width() <= 0.0 || rect.height() <= 0.0)
        return;

    static const QPixmap image = loadTagImage();
    if (image.isNull())
        return;

    painter->save();
    painter->setRenderHint(QPainter::SmoothPixmapTransform, true);
    const QSize fitted = image.size().scaled(rect.size().toSize(), Qt::KeepAspectRatio);
    const QRectF target(rect.center().x() - fitted.width() * 0.5,
                        rect.center().y() - fitted.height() * 0.5,
                        fitted.width(), fitted.height());
    painter->drawPixmap(target, image, QRectF(image.rect()));
    painter->restore();
}
