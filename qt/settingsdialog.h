#pragma once
#include "settings.h"
#include <QDialog>
#include <QLabel>
#include <QSet>

class QComboBox;
class QCheckBox;
class QLineEdit;
class QRadioButton;
class QVBoxLayout;

class SettingsDialog : public QDialog {
    Q_OBJECT
public:
    explicit SettingsDialog(AppSettings settings, QWidget *parent = nullptr);
    AppSettings settings() const { return m_settings; }

private slots:
    void onSourceChanged();
    void onBrowse();
    void onTestLocal();
    void onTestSmb();
    void onTestDb();
    void onShowPass(bool show);
    void onKoboAdd();
    void onThemeChanged(int i);
    void accept() override;

private:
    void refreshKoboRows();
    void capHeight();
    void collectSmb();
    void runProbe(QLabel *pill);
    AppSettings m_settings;
    QRadioButton *m_srcSmb;
    QRadioButton *m_srcLocal;
    QLineEdit *m_localDir;
    QLabel *m_localPass;
    QLineEdit *m_server;
    QLineEdit *m_share;
    QLineEdit *m_calibre;
    QLineEdit *m_user;
    QLineEdit *m_pass;
    QLineEdit *m_domain;
    QLabel *m_smbPass;
    QLabel *m_dbPass;
    QVBoxLayout *m_koboRows;
    QLineEdit *m_newKobo;
    QLabel *m_koboStatus;
    QComboBox *m_theme;
    QCheckBox *m_updateCheck;
    QCheckBox *m_diagCheck;
    QSet<QString> m_revealed;
};
