#include "bookmodel.h"

BookModel::BookModel(QObject *parent) : QAbstractListModel(parent) {}

void BookModel::setBooks(const QList<BookItem> &books) {
    beginResetModel();
    m_rows.clear();
    m_covers.clear();
    m_coversSmall.clear();
    for (const BookItem &b : books) {
        Row r;
        r.id = b.id;
        r.title = b.title;
        r.sub = b.author;
        r.coverPath = b.path;
        r.series = b.series;
        r.seriesIndex = b.seriesIndex;
        r.tags = b.tags;
        m_rows.append(r);
    }
    endResetModel();
}

void BookModel::setTiles(const QStringList &titles, const QStringList &subs,
                         const QList<qint64> &ids, const QStringList &coverPaths) {
    beginResetModel();
    m_rows.clear();
    m_covers.clear();
    m_coversSmall.clear();
    for (int i = 0; i < titles.size(); ++i) {
        Row r;
        r.id = i < ids.size() ? ids[i] : -1;
        r.title = titles[i];
        r.sub = i < subs.size() ? subs[i] : QString();
        r.coverPath = i < coverPaths.size() ? coverPaths[i] : QString();
        m_rows.append(r);
    }
    endResetModel();
}

void BookModel::setCover(const QString &path, const QImage &img) {
    if (path.isEmpty() || img.isNull())
        return;
    setCoverBatch({{path, img}});
}

void BookModel::setCoverBatch(const QList<QPair<QString, QImage>> &covers) {
    if (covers.isEmpty() || m_rows.isEmpty())
        return;
    for (const auto &c : covers) {
        if (!c.first.isEmpty() && !c.second.isNull()) {
            m_covers.insert(c.first, QPixmap::fromImage(c.second));
            m_coversSmall.insert(c.first,
                QPixmap::fromImage(c.second.scaledToHeight(
                    ListThumbHeight, Qt::SmoothTransformation)));
        }
    }
    // One range update for the whole view (coalesces layout passes).
    QModelIndex top = index(0), bottom = index(m_rows.size() - 1);
    emit dataChanged(top, bottom, {CoverRole, Qt::DecorationRole});
}

BookItem BookModel::bookAt(int row) const {
    BookItem b;
    if (row < 0 || row >= m_rows.size())
        return b;
    b.id = m_rows[row].id;
    b.title = m_rows[row].title;
    b.author = m_rows[row].sub;
    b.path = m_rows[row].coverPath;
    b.series = m_rows[row].series;
    b.seriesIndex = m_rows[row].seriesIndex;
    b.tags = m_rows[row].tags;
    return b;
}

qint64 BookModel::idAt(int row) const {
    return (row < 0 || row >= m_rows.size()) ? -1 : m_rows[row].id;
}

int BookModel::rowCount(const QModelIndex &parent) const {
    return parent.isValid() ? 0 : m_rows.size();
}

QVariant BookModel::data(const QModelIndex &index, int role) const {
    if (!index.isValid() || index.row() >= m_rows.size())
        return QVariant();
    const Row &r = m_rows[index.row()];
    switch (role) {
    case Qt::DisplayRole:
        return r.title;
    case TitleRole:
        return r.title;
    case AuthorRole:
        return r.sub;
    case PathRole:
        return r.coverPath;
    case IdRole:
        return r.id;
    case SeriesRole:
        return r.series;
    case SeriesIndexRole:
        return r.seriesIndex;
    case TagsRole:
        return r.tags;
    case CoverRole: {
        auto it = m_covers.find(r.coverPath);
        if (it != m_covers.end())
            return *it;
        if (!r.coverPath.isEmpty())
            const_cast<BookModel *>(this)->coverNeeded(r.coverPath);
        return QVariant();
    }
    case Qt::DecorationRole: {
        // List mode: row-height thumbnail via the standard decoration
        // path (same demand signal as the grid — one shared fetch).
        auto it = m_coversSmall.find(r.coverPath);
        if (it != m_coversSmall.end())
            return *it;
        if (!r.coverPath.isEmpty())
            const_cast<BookModel *>(this)->coverNeeded(r.coverPath);
        return QVariant();
    }
    }
    return QVariant();
}
