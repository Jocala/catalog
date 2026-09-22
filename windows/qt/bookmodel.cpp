#include "bookmodel.h"
// TEMP includes for the capped demand log.
#include <QDir>
#include <QFile>
#include <QTextStream>

BookModel::BookModel(QObject *parent) : QAbstractListModel(parent) {}

void BookModel::setBooks(const QList<BookItem> &books) {
    beginResetModel();
    m_rows.clear();
    m_covers.clear();
    for (const BookItem &b : books) {
        Row r;
        r.id = b.id;
        r.title = b.title;
        r.sub = b.author;
        r.coverPath = b.path;
        m_rows.append(r);
    }
    endResetModel();
}

void BookModel::setTiles(const QStringList &titles, const QStringList &subs,
                         const QList<qint64> &ids, const QStringList &coverPaths) {
    beginResetModel();
    m_rows.clear();
    m_covers.clear();
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
        if (!c.first.isEmpty() && !c.second.isNull())
            m_covers.insert(c.first, QPixmap::fromImage(c.second));
    }
    // One range update for the whole view (coalesces layout passes).
    QModelIndex top = index(0), bottom = index(m_rows.size() - 1);
    emit dataChanged(top, bottom, {CoverRole});
}

BookItem BookModel::bookAt(int row) const {
    BookItem b;
    if (row < 0 || row >= m_rows.size())
        return b;
    b.id = m_rows[row].id;
    b.title = m_rows[row].title;
    b.author = m_rows[row].sub;
    b.path = m_rows[row].coverPath;
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
    case CoverRole: {
        auto it = m_covers.find(r.coverPath);
        if (it != m_covers.end())
            return *it;
        if (!r.coverPath.isEmpty()) {
            // TEMP: capped demand log (revert with the rest).
            static int n = 0;
            if (n++ < 30) {
                QFile f(QDir::tempPath() + "/qt-cover.txt");
                if (f.open(QIODevice::Append | QIODevice::Text)) {
                    QTextStream s(&f);
                    s << "demand " << r.coverPath << "\n";
                }
            }
            const_cast<BookModel *>(this)->coverNeeded(r.coverPath);
        }
        return QVariant();
    }
    }
    return QVariant();
}
