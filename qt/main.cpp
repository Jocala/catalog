#include <QApplication>
#include <QIcon>
#include <QJsonDocument>
#include <QTextStream>
#include "ffi.h"
#include "ffijson.h"
#include "mainwindow.h"
#include "settings.h"
#include "theme.h"

static int selftest() {
    QJsonValue payload;
    QString err;
    if (!ffiOk(catalog_version(), payload, err)) {
        QTextStream(stderr) << "version failed: " << err << Qt::endl;
        return 1;
    }
    QTextStream(stdout) << QJsonDocument(payload.toObject()).toJson(QJsonDocument::Compact);
    return 0;
}

int main(int argc, char **argv) {
    for (int i = 1; i < argc; ++i) {
        if (QString(argv[i]) == "--selftest")
            return selftest();
    }
    QApplication app(argc, argv);
    app.setApplicationName("Jocala Catalog");
    app.setOrganizationName("Jocala Software");
    app.setWindowIcon(QIcon(":/catalog/appicon.ico"));
    applyTheme(AppSettings::load().theme);
    MainWindow w;
    w.show();
    return app.exec();
}
