#pragma once
// List-mode rows: thumbnail + two-line text (title semibold, author or
// count muted). Clicked rows paint identically to unclicked ones (house
// rule — no selection fill); hover tint stays as the position cue.
#include <QStyledItemDelegate>

class ListDelegate : public QStyledItemDelegate {
    Q_OBJECT
public:
    explicit ListDelegate(QObject *parent = nullptr);
    void paint(QPainter *p, const QStyleOptionViewItem &opt,
               const QModelIndex &idx) const override;
    QSize sizeHint(const QStyleOptionViewItem &opt,
                   const QModelIndex &idx) const override;
    static const int RowHeight = 136;
};
