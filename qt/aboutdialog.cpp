#include "aboutdialog.h"
#include "updatecheck.h"
#include "version.h"
#include <QDesktopServices>
#include <QDialogButtonBox>
#include <QLabel>
#include <QPushButton>
#include <QUrl>
#include <QVBoxLayout>

namespace {
const char *kPayPal =
    "https://www.paypal.com/cgi-bin/webscr?cmd=_s-xclick&hosted_button_id=GKZMW456H6E5W";
}

AboutDialog::AboutDialog(QWidget *parent) : QDialog(parent) {
    setWindowTitle("About Jocala Catalog");
    setFixedSize(380, 340);
    QVBoxLayout *top = new QVBoxLayout(this);
    top->setContentsMargins(20, 20, 20, 20);
    QLabel *name = new QLabel("Jocala Catalog", this);
    name->setAlignment(Qt::AlignCenter);
    QFont f = name->font();
    f.setPointSize(15);
    f.setBold(true);
    name->setFont(f);
    top->addWidget(name);
    QLabel *ver = new QLabel("Version " + kCatalogVersion, this);
    ver->setAlignment(Qt::AlignCenter);
    ver->setStyleSheet("color: gray");
    top->addWidget(ver);
    QLabel *site = new QLabel("<a href=\"https://www.jocala.com\">jocala.com</a>", this);
    site->setAlignment(Qt::AlignCenter);
    site->setOpenExternalLinks(true);
    top->addWidget(site);
    QLabel *thanks = new QLabel("Donations defray server costs and fund development.", this);
    thanks->setAlignment(Qt::AlignCenter);
    thanks->setWordWrap(true);
    top->addWidget(thanks);
    QPushButton *donate = new QPushButton(this);
    donate->setIcon(QIcon(":/catalog/donatel.png"));
    donate->setIconSize(QSize(190, 20));
    donate->setFlat(true);
    donate->setCursor(Qt::PointingHandCursor);
    donate->setToolTip("Donate via PayPal");
    connect(donate, &QPushButton::clicked, this, []() {
        QDesktopServices::openUrl(QUrl(kPayPal));
    });
    top->addWidget(donate, 0, Qt::AlignCenter);
    QPushButton *updates = new QPushButton("Check for Updates", this);
    connect(updates, &QPushButton::clicked, this, [this]() {
        // Heap + parented: the async reply must outlive this lambda.
        auto *checker = new UpdateChecker(this);
        checker->check(this, false);
    });
    top->addWidget(updates, 0, Qt::AlignCenter);
    QDialogButtonBox *buttons = new QDialogButtonBox(QDialogButtonBox::Close, this);
    connect(buttons, &QDialogButtonBox::rejected, this, &AboutDialog::accept);
    top->addWidget(buttons, 0, Qt::AlignCenter);
}
