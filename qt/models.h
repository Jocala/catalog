#pragma once
// Plain models parsed from FFI JSON (serde keys verbatim).
#include <QJsonObject>
#include <QString>
#include <QStringList>

struct BookItem {
    qint64 id = -1;
    QString title;
    QString author;
    QString path;
    QString coverHash;
    bool hasCover = false;
    QString authorSort;
    QString timestamp;
    QString series;
    double seriesIndex = 0;
    QString tags;
    static BookItem fromJson(const QJsonObject &o);
};

struct AuthorItem {
    qint64 id = -1;
    QString name;
    QString bookCount;
    QString firstPath;
    static AuthorItem fromJson(const QJsonObject &o);
};

struct SeriesItem {
    qint64 id = -1;
    QString name;
    QString bookCount;
    QString firstPath;
    static SeriesItem fromJson(const QJsonObject &o);
};

struct TagItem {
    qint64 id = -1;
    QString name;
    QString bookCount;
    static TagItem fromJson(const QJsonObject &o);
};

struct DetailItem {
    qint64 id = -1;
    QString title;
    QString author;
    QString series;
    double seriesIndex = 0;
    QString comments;
    QString tags;
    QString publisher;
    QString isbn;
    static DetailItem fromJson(const QJsonObject &o);
};

QList<BookItem> parseBooks(const QJsonValue &v);
QList<AuthorItem> parseAuthors(const QJsonValue &v);
QList<SeriesItem> parseSeries(const QJsonValue &v);
QList<TagItem> parseTags(const QJsonValue &v);
