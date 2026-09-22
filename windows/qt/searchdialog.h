#pragma once
#include "settings.h"
#include <QDialog>
#include <QJsonObject>

class QCheckBox;
class QComboBox;
class QLineEdit;

class SearchDialog : public QDialog {
    Q_OBJECT
public:
    explicit SearchDialog(const AppSettings &settings, QWidget *parent = nullptr);
    QJsonObject params() const;

private:
    void loadLists();

    QLineEdit *m_query;
    QLineEdit *m_title;
    QLineEdit *m_author;
    QComboBox *m_series;
    QComboBox *m_tags;
    QCheckBox *m_expand;
    QString m_cfg;
};
