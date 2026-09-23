#pragma once
// Update check, modeled on adblink's version check: fetch the current
// version from jocala.com, compare against kCatalogVersion, and offer
// the download page when a newer release is ready.
#include <QObject>

class QWidget;
class QNetworkAccessManager;

class UpdateChecker : public QObject {
    Q_OBJECT
public:
    explicit UpdateChecker(QObject *parent = nullptr);

    // Fetch cversion.txt and offer the update when newer. quiet=true
    // (startup) stays silent unless an update is actually ready; manual
    // checks report errors and the up-to-date state too.
    void check(QWidget *parentWidget, bool quiet);

    // Pure compare, unit-tested: any trimmed difference means newer
    // (same semantics as adblink — including a version rollback).
    static bool versionIsNewer(const QString &current, const QString &fetched);

private:
    QNetworkAccessManager *m_net;
};
