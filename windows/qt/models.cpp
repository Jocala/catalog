#include "models.h"
#include <QJsonArray>

static QString str(const QJsonObject &o, const char *k) {
    return o.value(QLatin1String(k)).toString();
}

BookItem BookItem::fromJson(const QJsonObject &o) {
    BookItem b;
    b.id = (qint64)o.value("id").toDouble(-1);
    b.title = str(o, "title");
    b.author = str(o, "author");
    b.path = str(o, "path");
    b.coverHash = str(o, "cover_hash");
    b.hasCover = o.value("has_cover").toBool(!b.coverHash.isEmpty());
    b.authorSort = str(o, "author_sort");
    b.timestamp = str(o, "timestamp");
    return b;
}

AuthorItem AuthorItem::fromJson(const QJsonObject &o) {
    AuthorItem a;
    a.id = (qint64)o.value("id").toDouble(-1);
    a.name = str(o, "name");
    a.bookCount = QString::number((qint64)o.value("book_count").toDouble(0)) + " books";
    a.firstPath = str(o, "first_book_path");
    return a;
}

SeriesItem SeriesItem::fromJson(const QJsonObject &o) {
    SeriesItem s;
    s.id = (qint64)o.value("id").toDouble(-1);
    s.name = str(o, "name");
    s.bookCount = QString::number((qint64)o.value("book_count").toDouble(0)) + " books";
    s.firstPath = str(o, "first_book_path");
    return s;
}

TagItem TagItem::fromJson(const QJsonObject &o) {
    TagItem t;
    t.id = (qint64)o.value("id").toDouble(-1);
    t.name = str(o, "name");
    t.bookCount = QString::number((qint64)o.value("book_count").toDouble(0)) + " books";
    return t;
}

DetailItem DetailItem::fromJson(const QJsonObject &o) {
    DetailItem d;
    d.id = (qint64)o.value("id").toDouble(-1);
    d.title = str(o, "title");
    d.author = str(o, "author");
    d.series = str(o, "series");
    d.seriesIndex = o.value("series_index").toDouble(0);
    d.comments = str(o, "comments");
    d.tags = str(o, "tags");
    d.publisher = str(o, "publisher");
    d.isbn = str(o, "isbn");
    return d;
}

QList<BookItem> parseBooks(const QJsonValue &v) {
    QList<BookItem> out;
    for (const QJsonValue &e : v.toArray())
        out.append(BookItem::fromJson(e.toObject()));
    return out;
}

QList<AuthorItem> parseAuthors(const QJsonValue &v) {
    QList<AuthorItem> out;
    for (const QJsonValue &e : v.toArray())
        out.append(AuthorItem::fromJson(e.toObject()));
    return out;
}

QList<SeriesItem> parseSeries(const QJsonValue &v) {
    QList<SeriesItem> out;
    for (const QJsonValue &e : v.toArray())
        out.append(SeriesItem::fromJson(e.toObject()));
    return out;
}

QList<TagItem> parseTags(const QJsonValue &v) {
    QList<TagItem> out;
    for (const QJsonValue &e : v.toArray())
        out.append(TagItem::fromJson(e.toObject()));
    return out;
}
