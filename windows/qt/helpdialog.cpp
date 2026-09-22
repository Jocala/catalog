#include "helpdialog.h"
#include <QDialogButtonBox>
#include <QTextBrowser>
#include <QUrl>
#include <QVBoxLayout>

HelpDialog::HelpDialog(QWidget *parent) : QDialog(parent) {
    setWindowTitle("Jocala Catalog Help");
    resize(760, 600);
    QVBoxLayout *top = new QVBoxLayout(this);
    QTextBrowser *view = new QTextBrowser(this);
    view->setOpenExternalLinks(true);
    // In-page Close link (catalog:close) handled below.
    view->setSource(QUrl("qrc:/catalog/help.html"));
    connect(view, &QTextBrowser::anchorClicked, this, [view](const QUrl &url) {
        if (url.toString() == "catalog:close") {
            QWidget *w = view->parentWidget();
            while (w && !qobject_cast<QDialog *>(w))
                w = w->parentWidget();
            if (QDialog *d = qobject_cast<QDialog *>(w))
                d->accept();
        } else {
            view->setSource(url);
        }
    });
    top->addWidget(view);
    QDialogButtonBox *buttons = new QDialogButtonBox(QDialogButtonBox::Close, this);
    connect(buttons, &QDialogButtonBox::rejected, this, &HelpDialog::accept);
    top->addWidget(buttons);
}
