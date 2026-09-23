// UpdateChecker::versionIsNewer: pure compare, no network.
#include "../updatecheck.h"
#include <QTest>

class TstUpdateCheck : public QObject {
    Q_OBJECT
private slots:
    void newer() {
        QVERIFY(UpdateChecker::versionIsNewer("1.0", "1.1"));
        QVERIFY(UpdateChecker::versionIsNewer("1.0", "  1.1\n"));
    }
    void current() {
        QVERIFY(!UpdateChecker::versionIsNewer("1.0", "1.0"));
        QVERIFY(!UpdateChecker::versionIsNewer("1.0", "1.0\n"));
    }
    void emptyFetchIsNotNewer() {
        QVERIFY(!UpdateChecker::versionIsNewer("1.0", ""));
        QVERIFY(!UpdateChecker::versionIsNewer("1.0", "  \n "));
    }
};

QTEST_MAIN(TstUpdateCheck)
#include "tst_updatecheck.moc"
