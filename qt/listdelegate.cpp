#include "listdelegate.h"
#include "bookmodel.h"
#include "tagicon.h"
#include <QPainter>

ListDelegate::ListDelegate(QObject *parent) : QStyledItemDelegate(parent) {}

QSize ListDelegate::sizeHint(const QStyleOptionViewItem &opt,
                             const QModelIndex &) const {
    return QSize(opt.rect.width(), RowHeight);
}

void ListDelegate::paint(QPainter *p, const QStyleOptionViewItem &opt,
                         const QModelIndex &idx) const {
    p->save();
    if (opt.state & QStyle::State_MouseOver)
        p->fillRect(opt.rect, opt.palette.alternateBase());

    const int margin = 8;
    const int thumbBox = 180;
    int textX = opt.rect.x() + margin + thumbBox + margin;
    int textW = opt.rect.right() - textX - margin;

    if (idx.data(BookModel::TagTileRole).toBool()) {
        paintTagIcon(p, QRectF(opt.rect.x() + margin, opt.rect.y(),
                               thumbBox, RowHeight));
    } else {
        QPixmap thumb = idx.data(Qt::DecorationRole).value<QPixmap>();
        if (!thumb.isNull()) {
            int tw = qMin(thumb.width(), thumbBox);
            int th = qMin(thumb.height(), thumbBox);
            int tx = opt.rect.x() + margin + (thumbBox - tw) / 2;
            int ty = opt.rect.y() + (RowHeight - th) / 2;
            p->drawPixmap(tx, ty, tw, th, thumb);
        }
    }

    QString title = idx.data(BookModel::TitleRole).toString();
    QString sub = idx.data(BookModel::AuthorRole).toString();
    QString tags = idx.data(BookModel::TagsRole).toString();
    QString series = idx.data(BookModel::SeriesRole).toString();
    double seriesIndex = idx.data(BookModel::SeriesIndexRole).toDouble();

    QFont titleFont = p->font();
    titleFont.setWeight(QFont::DemiBold);
    p->setFont(titleFont);
    p->setPen(opt.palette.text().color());
    QFontMetrics tfm(titleFont);
    QString titleElided = tfm.elidedText(title, Qt::ElideRight, textW);
    p->drawText(textX, opt.rect.y() + 78, textW, 22,
                Qt::AlignLeft | Qt::AlignVCenter, titleElided);

    QFont subFont = p->font();
    subFont.setPointSize(qMax(8, subFont.pointSize() - 1));
    p->setFont(subFont);
    p->setPen(Qt::gray);
    QFontMetrics sfm(subFont);
    QString subElided = sfm.elidedText(sub, Qt::ElideRight, textW);
    p->drawText(textX, opt.rect.y() + 100, textW, 18,
                Qt::AlignLeft | Qt::AlignVCenter, subElided);
    if (!tags.isEmpty()) {
        QString tagsElided = sfm.elidedText(tags, Qt::ElideRight, textW);
        p->drawText(textX, opt.rect.y() + 122, textW, 18,
                    Qt::AlignLeft | Qt::AlignVCenter, tagsElided);
    }
    if (!series.isEmpty()) {
        QString line = QString("%1 #%2").arg(series).arg(seriesIndex);
        QString lineElided = sfm.elidedText(line, Qt::ElideRight, textW);
        p->drawText(textX, opt.rect.y() + 140, textW, 18,
                    Qt::AlignLeft | Qt::AlignVCenter, lineElided);
    }
    p->restore();
}
