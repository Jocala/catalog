#include "listdelegate.h"
#include "bookmodel.h"
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
    const int thumbBox = 120;
    int textX = opt.rect.x() + margin + thumbBox + margin;
    int textW = opt.rect.right() - textX - margin;

    QPixmap thumb = idx.data(Qt::DecorationRole).value<QPixmap>();
    if (!thumb.isNull()) {
        int tw = qMin(thumb.width(), thumbBox);
        int th = qMin(thumb.height(), thumbBox);
        int tx = opt.rect.x() + margin + (thumbBox - tw) / 2;
        int ty = opt.rect.y() + (RowHeight - th) / 2;
        p->drawPixmap(tx, ty, tw, th, thumb);
    }

    QString title = idx.data(BookModel::TitleRole).toString();
    QString sub = idx.data(BookModel::AuthorRole).toString();

    QFont titleFont = p->font();
    titleFont.setWeight(QFont::DemiBold);
    p->setFont(titleFont);
    p->setPen(opt.palette.text().color());
    QFontMetrics tfm(titleFont);
    QString titleElided = tfm.elidedText(title, Qt::ElideRight, textW);
    p->drawText(textX, opt.rect.y() + 48, textW, 22,
                Qt::AlignLeft | Qt::AlignVCenter, titleElided);

    QFont subFont = p->font();
    subFont.setPointSize(qMax(8, subFont.pointSize() - 1));
    p->setFont(subFont);
    p->setPen(Qt::gray);
    QFontMetrics sfm(subFont);
    QString subElided = sfm.elidedText(sub, Qt::ElideRight, textW);
    p->drawText(textX, opt.rect.y() + 70, textW, 18,
                Qt::AlignLeft | Qt::AlignVCenter, subElided);
    p->restore();
}
