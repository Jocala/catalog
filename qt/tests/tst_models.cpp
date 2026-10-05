#include "tst_models.h"

static QJsonArray parseArray(const char *json) {
    return QJsonDocument::fromJson(QByteArray(json)).array();
}

static QJsonObject parseObject(const char *json) {
    return QJsonDocument::fromJson(QByteArray(json)).object();
}

void TstModels::books() {
    QList<BookItem> books = parseBooks(parseArray(R"([{"id":1,"title":"Emma","author":"Austen, Jane","path":"smb://h/s/Austen/Emma","has_cover":true,"cover_hash":"ab"}])"));
    QCOMPARE(books.size(), 1);
    QCOMPARE(books[0].title, QString("Emma"));
    QCOMPARE(books[0].path, QString("smb://h/s/Austen/Emma"));
    QVERIFY(books[0].hasCover);
}

void TstModels::summaries() {
    QList<AuthorItem> authors = parseAuthors(parseArray(R"([{"id":7,"name":"Austen, Jane","sort":"Austen, Jane","book_count":6,"first_book_path":"p"}])"));
    QCOMPARE(authors.size(), 1);
    QCOMPARE(authors[0].bookCount, QString("6 books"));
}

void TstModels::detail() {
    DetailItem d = DetailItem::fromJson(parseObject(R"({"id":2,"title":"Dune","author":"Herbert, Frank","series":"Dune","series_index":1.0,"tags":"sci-fi","publisher":"Chilton","isbn":"978","pubdate":"","timestamp":"","author_sort":"Herbert, Frank"})"));
    QCOMPARE(d.title, QString("Dune"));
    QCOMPARE(d.seriesIndex, 1.0);
    QCOMPARE(d.isbn, QString("978"));
}

void TstModels::seriesInTitle() {
    // Series books carry their number with the title; others are bare.
    QCOMPARE(displayTitle("Emma", "Classics", 1.0), QString("Emma #1"));
    QCOMPARE(displayTitle("Hound", "Holmes", 1.5), QString("Hound #1.5"));
    QCOMPARE(displayTitle("Emma", "", 0.0), QString("Emma"));
    QCOMPARE(displayTitle("Emma", "", 1.0), QString("Emma"));
    QCOMPARE(displayTitle("Emma", "Classics", 0.0), QString("Emma"));
}

QTEST_MAIN(TstModels)
#include "tst_models.moc"
