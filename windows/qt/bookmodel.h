#pragma once
// Gallery model: books with async cover role; tile kinds share one model.
// Covers are stored as QPixmap (converted once) so paint never converts.
#include "models.h"
#include <QAbstractListModel>
#include <QPixmap>
#include <QSet>

class BookModel : public QAbstractListModel {
    Q_OBJECT
public:
    enum Roles { TitleRole = Qt::UserRole + 1, AuthorRole, PathRole, CoverRole, IdRole };

    explicit BookModel(QObject *parent = nullptr);

    void setBooks(const QList<BookItem> &books);
    void setTiles(const QStringList &titles, const QStringList &subs,
                  const QList<qint64> &ids, const QStringList &coverPaths);
    void setCover(const QString &path, const QImage &img);
    void setCoverBatch(const QList<QPair<QString, QImage>> &covers);
    BookItem bookAt(int row) const;
    qint64 idAt(int row) const;

    int rowCount(const QModelIndex &parent = QModelIndex()) const override;
    QVariant data(const QModelIndex &index, int role = Qt::DisplayRole) const override;

signals:
    void coverNeeded(const QString &path);

private:
    struct Row {
        qint64 id = -1;
        QString title;
        QString sub;
        QString coverPath;
    };
    QList<Row> m_rows;
    QHash<QString, QPixmap> m_covers;
};
