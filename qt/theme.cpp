#include "theme.h"
#include <QApplication>
#include <QPalette>
#include <QStyle>
#include <QStyleFactory>
#include <QStyleHints>

void applyTheme(int preference) {
    QApplication::setStyle(QStyleFactory::create("Fusion"));
    int pref = preference;
    if (pref == 0) {
#if QT_VERSION >= QT_VERSION_CHECK(6, 5, 0)
        pref = (QGuiApplication::styleHints()->colorScheme() == Qt::ColorScheme::Dark) ? 2 : 1;
#else
        pref = 1;
#endif
    }
    if (pref == 2) {
        QPalette dark;
        dark.setColor(QPalette::Window, QColor(30, 30, 30));
        dark.setColor(QPalette::WindowText, Qt::white);
        dark.setColor(QPalette::Base, QColor(18, 18, 18));
        dark.setColor(QPalette::AlternateBase, QColor(30, 30, 30));
        dark.setColor(QPalette::Text, Qt::white);
        dark.setColor(QPalette::Button, QColor(45, 45, 45));
        dark.setColor(QPalette::ButtonText, Qt::white);
        dark.setColor(QPalette::BrightText, Qt::red);
        dark.setColor(QPalette::Link, QColor(42, 130, 218));
        dark.setColor(QPalette::Highlight, QColor(42, 130, 218));
        dark.setColor(QPalette::HighlightedText, Qt::white);
        QApplication::setPalette(dark);
    } else {
        QApplication::setPalette(QApplication::style()->standardPalette());
    }
}
