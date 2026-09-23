#include "updatecheck.h"
#include "version.h"
#include <QAbstractButton>
#include <QDesktopServices>
#include <QDialog>
#include <QDialogButtonBox>
#include <QLabel>
#include <QMessageBox>
#include <QNetworkAccessManager>
#include <QNetworkReply>
#include <QNetworkRequest>
#include <QUrl>
#include <QVBoxLayout>

namespace {
const char *kVersionUrl = "https://www.jocala.com/cversion.txt";
const char *kCatalogPage = "https://www.jocala.com/catalog/";
const char *kChangelogUrl = "https://www.jocala.com/catalog/changelog.txt";
}

UpdateChecker::UpdateChecker(QObject *parent)
    : QObject(parent), m_net(new QNetworkAccessManager(this)) {}

bool UpdateChecker::versionIsNewer(const QString &current, const QString &fetched) {
    QString f = fetched.trimmed();
    return !f.isEmpty() && f != current.trimmed();
}

void UpdateChecker::check(QWidget *parentWidget, bool quiet) {
    QNetworkReply *reply = m_net->get(QNetworkRequest(QUrl(kVersionUrl)));
    connect(reply, &QNetworkReply::finished, this, [this, reply, parentWidget, quiet]() {
        reply->deleteLater();
        if (reply->error() != QNetworkReply::NoError) {
            if (quiet)
                return; // startup: offline is not worth a popup
            QMessageBox::warning(parentWidget, "Check for Updates",
                                 QString("Could not reach %1:\n%2")
                                     .arg(kVersionUrl, reply->errorString()));
            return;
        }
        QString fetched = QString::fromUtf8(reply->readAll());
        if (!versionIsNewer(kCatalogVersion, fetched)) {
            if (quiet)
                return;
            QMessageBox::information(parentWidget, "Check for Updates",
                                     QString("Jocala Catalog %1 is up to date.")
                                         .arg(kCatalogVersion));
            return;
        }
        QDialog dlg(parentWidget);
        dlg.setWindowTitle("Catalog Update");
        QVBoxLayout *lay = new QVBoxLayout(&dlg);
        lay->addWidget(new QLabel(
            QString("Jocala Catalog version %1 is ready. Download?").arg(fetched.trimmed()),
            &dlg));
        QDialogButtonBox *box = new QDialogButtonBox(&dlg);
        box->addButton("Yes", QDialogButtonBox::AcceptRole);
        box->addButton("No", QDialogButtonBox::RejectRole);
        box->addButton("Changelog", QDialogButtonBox::ActionRole);
        lay->addWidget(box);
        connect(box, &QDialogButtonBox::accepted, [&dlg]() {
            QDesktopServices::openUrl(QUrl(kCatalogPage));
            dlg.accept();
        });
        connect(box, &QDialogButtonBox::rejected, &dlg, &QDialog::reject);
        connect(box, &QDialogButtonBox::clicked, [&](QAbstractButton *b) {
            if (box->buttonRole(b) == QDialogButtonBox::ActionRole) {
                QDesktopServices::openUrl(QUrl(kChangelogUrl));
                dlg.close();
            }
        });
        dlg.exec();
    });
}
