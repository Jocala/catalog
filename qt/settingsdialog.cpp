#include "settingsdialog.h"
#include "ffi.h"
#include "ffijson.h"
#include "kobojob.h"
#include <QCheckBox>
#include <QComboBox>
#include <QDialogButtonBox>
#include <QFileDialog>
#include <QFontMetrics>
#include <QFormLayout>
#include <QFutureWatcher>
#include <QGroupBox>
#include <QHBoxLayout>
#include <QJsonDocument>
#include <QJsonObject>
#include <QLabel>
#include <QLineEdit>
#include <QMessageBox>
#include <QPushButton>
#include <QRadioButton>
#include <QScrollArea>
#include <QVBoxLayout>
#include <QMessageBox>
#include <QPushButton>
#include <QRadioButton>
#include <QScrollArea>
#include <QScreen>
#include <QtConcurrent>
#include <QVBoxLayout>

namespace {
void stylePass(QLabel *l, const QString &text, bool ok) {
    l->setText(text);
    l->setStyleSheet(QString("font-weight: bold; color: %1").arg(ok ? "green" : "red"));
}

// Reserve the pill width up front (bold "Fail" is the widest state) so
// showing Pass/Fail never reflows the row.
void fixPillWidth(QLabel *pill) {
    QFont f = pill->font();
    f.setBold(true);
    pill->setMinimumWidth(QFontMetrics(f).horizontalAdvance("Fail") + 8);
}
// Blocking library probe off the thread: returns (ok, detail).
QPair<bool, QString> probeLibrary(const QString &cfg) {
    QByteArray cfgB = cfg.toUtf8();
    QJsonValue payload;
    QString err;
    if (!ffiOk(catalog_open_count(cfgB.constData()), payload, err))
        return qMakePair(false, err);
    return qMakePair(true, QString("%1 books").arg(payload.toObject().value("books").toInt()));
}
}

SettingsDialog::SettingsDialog(AppSettings settings, QWidget *parent)
    : QDialog(parent), m_settings(settings) {
    setWindowTitle("Settings");
    // Fixed width (measured against the live dialog): the Kobo rows'
    // fixed-width controls must fit without a horizontal scrollbar.
    // Height stays flexible — the dialog grows with Kobo rows (capped)
    // while Save/Cancel sit outside the scroll area, always visible.
    resize(778, 640);
    setFixedWidth(778);
    QVBoxLayout *outer = new QVBoxLayout(this);
    QScrollArea *scroll = new QScrollArea(this);
    scroll->setWidgetResizable(true);
    scroll->setHorizontalScrollBarPolicy(Qt::ScrollBarAlwaysOff);
    QWidget *body = new QWidget(this);
    QVBoxLayout *top = new QVBoxLayout(body);

    QGroupBox *srcGroup = new QGroupBox("Library Source", body);
    QVBoxLayout *srcLay = new QVBoxLayout(srcGroup);
    QHBoxLayout *srcRow = new QHBoxLayout();
    m_srcSmb = new QRadioButton("SMB", srcGroup);
    m_srcLocal = new QRadioButton("Local", srcGroup);
    srcRow->addWidget(m_srcSmb);
    srcRow->addWidget(m_srcLocal);
    srcRow->addStretch();
    srcLay->addLayout(srcRow);
    QHBoxLayout *localRow = new QHBoxLayout();
    localRow->addWidget(new QLabel("Local Calibre Path", srcGroup));
    m_localDir = new QLineEdit(srcGroup);
    m_localDir->setMinimumWidth(240);
    m_localDir->setPlaceholderText("Choose a Calibre folder…");
    m_localDir->setText(m_settings.localDir);
    QPushButton *browseBtn = new QPushButton("Browse", srcGroup);
    connect(browseBtn, &QPushButton::clicked, this, &SettingsDialog::onBrowse);
    QPushButton *testLocalBtn = new QPushButton("Test", srcGroup);
    connect(testLocalBtn, &QPushButton::clicked, this, &SettingsDialog::onTestLocal);
    m_localPass = new QLabel(srcGroup);
    fixPillWidth(m_localPass);
    localRow->addWidget(m_localDir);
    localRow->addWidget(browseBtn);
    localRow->addWidget(testLocalBtn);
    localRow->addWidget(m_localPass);
    localRow->addStretch();
    srcLay->addLayout(localRow);
    top->addWidget(srcGroup);

    QGroupBox *smbGroup = new QGroupBox("SMB", body);
    QFormLayout *smbForm = new QFormLayout(smbGroup);
    const SmbServer *srv = m_settings.primaryServer();
    m_server = new QLineEdit(smbGroup);
    m_server->setPlaceholderText("hostname or IP");
    m_server->setFont(QFont("Consolas"));
    if (srv) m_server->setText(srv->host);
    QHBoxLayout *smbTestRow = new QHBoxLayout();
    smbTestRow->addWidget(m_server);
    QPushButton *testSmbBtn = new QPushButton("Test SMB", smbGroup);
    connect(testSmbBtn, &QPushButton::clicked, this, &SettingsDialog::onTestSmb);
    m_smbPass = new QLabel(smbGroup);
    fixPillWidth(m_smbPass);
    smbTestRow->addWidget(testSmbBtn);
    smbTestRow->addWidget(m_smbPass);
    smbForm->addRow("SMB Server", smbTestRow);
    m_share = new QLineEdit(smbGroup);
    m_share->setPlaceholderText("share name");
    if (srv) m_share->setText(srv->shares.value(0).name);
    smbForm->addRow("Share", m_share);
    m_calibre = new QLineEdit(smbGroup);
    m_calibre->setPlaceholderText("folder of metadata.db, e.g. calibre/");
    m_calibre->setToolTip("Folder or database file inside the share, e.g. calibre or calibre/metadata.db");
    if (srv && !srv->shares.isEmpty()) m_calibre->setText(srv->shares.first().calibrePath);
    QHBoxLayout *dbTestRow = new QHBoxLayout();
    dbTestRow->addWidget(m_calibre);
    QPushButton *testDbBtn = new QPushButton("Test Calibre", smbGroup);
    connect(testDbBtn, &QPushButton::clicked, this, &SettingsDialog::onTestDb);
    m_dbPass = new QLabel(smbGroup);
    fixPillWidth(m_dbPass);
    dbTestRow->addWidget(testDbBtn);
    dbTestRow->addWidget(m_dbPass);
    smbForm->addRow("SMB Calibre Path", dbTestRow);
    m_user = new QLineEdit(smbGroup);
    m_user->setPlaceholderText("username");
    if (srv) m_user->setText(srv->user);
    smbForm->addRow("User", m_user);
    m_pass = new QLineEdit(smbGroup);
    m_pass->setEchoMode(QLineEdit::Password);
    m_pass->setPlaceholderText("Required");
    if (srv) m_pass->setText(m_settings.smbPassword(srv->host));
    QPushButton *showBtn = new QPushButton("Show", smbGroup);
    showBtn->setCheckable(true);
    connect(showBtn, &QPushButton::toggled, this, &SettingsDialog::onShowPass);
    QHBoxLayout *passRow = new QHBoxLayout();
    passRow->addWidget(m_pass);
    passRow->addWidget(showBtn);
    smbForm->addRow("Password", passRow);
    m_domain = new QLineEdit(smbGroup);
    m_domain->setPlaceholderText("optional");
    if (srv) m_domain->setText(srv->domain);
    smbForm->addRow("Domain", m_domain);
    top->addWidget(smbGroup);

    QGroupBox *koboGroup = new QGroupBox("Kobo", body);
    QVBoxLayout *koboLay = new QVBoxLayout(koboGroup);
    m_koboRows = new QVBoxLayout();
    koboLay->addLayout(m_koboRows);
    QHBoxLayout *addRow = new QHBoxLayout();
    m_newKobo = new QLineEdit(koboGroup);
    m_newKobo->setPlaceholderText("Kobo IP address");
    m_newKobo->setFont(QFont("Consolas"));
    m_newKobo->setFixedWidth(150);
    QPushButton *addBtn = new QPushButton("Add", koboGroup);
    connect(addBtn, &QPushButton::clicked, this, &SettingsDialog::onKoboAdd);
    QPushButton *fixBtn = new QPushButton("Install stacking fix", koboGroup);
    fixBtn->setToolTip("Install the KOReader stacking-fix on the active Kobo (needs key login)");
    connect(fixBtn, &QPushButton::clicked, this, [this, fixBtn]() {
        // Active Kobo: explicit default wins, else the single configured IP.
        QString ip = m_settings.koboIp.trimmed();
        if (ip.isEmpty() && m_settings.koboIps.size() == 1)
            ip = m_settings.koboIps.first().trimmed();
        if (ip.isEmpty()) {
            m_koboStatus->setStyleSheet("color: gray");
            m_koboStatus->setText("No active Kobo — add an IP first.");
            return;
        }
        fixBtn->setEnabled(false);
        m_koboStatus->setStyleSheet("color: gray");
        m_koboStatus->setText(QString("Installing stacking fix on %1…").arg(ip));
        QFutureWatcher<QJsonObject> *w = new QFutureWatcher<QJsonObject>(this);
        connect(w, &QFutureWatcher<QJsonObject>::finished, this, [this, ip, fixBtn, w]() {
            QJsonObject o = w->result();
            w->deleteLater();
            fixBtn->setEnabled(true);
            QString state = o.value("state").toString();
            if (state == "installed" || state == "already") {
                m_settings.koboHandoffPromptDone = true;
                m_koboStatus->setStyleSheet("color: green");
                m_koboStatus->setText(state == "already"
                    ? QString("Stacking fix already on %1").arg(ip)
                    : QString("Stacking fix installed on %1").arg(ip));
            } else {
                QString msg = o.value("status").toString() == "failed"
                    ? o.value("message").toString()
                    : o.value("output").toString();
                m_koboStatus->setStyleSheet("color: red");
                m_koboStatus->setText(
                    QString("Stacking fix failed on %1: %2").arg(ip, msg.left(120)));
            }
        });
        w->setFuture(QtConcurrent::run([ip]() { return KoboJob::handoffEnsure(ip); }));
    });
    addRow->addWidget(m_newKobo);
    addRow->addWidget(addBtn);
    addRow->addWidget(fixBtn);
    addRow->addStretch();
    koboLay->addLayout(addRow);
    QLabel *koboHint = new QLabel(
        "Star selects default for Read on Kobo. IP + password edit in place; password is optional. Test checks "
        "the connection. Fix installs the KOReader stacking-fix on the active Kobo (needs key login).",
        koboGroup);
    koboHint->setWordWrap(true);
    koboHint->setStyleSheet("color: gray");
    koboLay->addWidget(koboHint);
    m_koboStatus = new QLabel(koboGroup);
    m_koboStatus->setStyleSheet("color: gray");
    m_koboStatus->setWordWrap(true);
    koboLay->addWidget(m_koboStatus);
    top->addWidget(koboGroup);

    QGroupBox *genGroup = new QGroupBox("General", body);
    QHBoxLayout *genLay = new QHBoxLayout(genGroup);
    genLay->addWidget(new QLabel("Theme", genGroup));
    m_theme = new QComboBox(genGroup);
    m_theme->addItems({"System", "Light", "Dark"});
    m_theme->setCurrentIndex(qBound(0, m_settings.theme, 2));
    connect(m_theme, QOverload<int>::of(&QComboBox::currentIndexChanged),
            this, &SettingsDialog::onThemeChanged);
    genLay->addWidget(m_theme);
    genLay->addStretch();
    top->addWidget(genGroup);
    top->addStretch();
    scroll->setWidget(body);
    outer->addWidget(scroll);
    QDialogButtonBox *buttons = new QDialogButtonBox(
        QDialogButtonBox::Save | QDialogButtonBox::Cancel, this);
    connect(buttons, &QDialogButtonBox::accepted, this, &SettingsDialog::accept);
    connect(buttons, &QDialogButtonBox::rejected, this, &SettingsDialog::reject);
    outer->addWidget(buttons);

    if (m_settings.librarySource == "local") {
        m_srcLocal->setChecked(true);
    } else {
        m_srcSmb->setChecked(true);
    }
    connect(m_srcSmb, &QRadioButton::toggled, this, &SettingsDialog::onSourceChanged);
    onSourceChanged();
    refreshKoboRows();
    capHeight();
}

// Grow with content (Kobo rows) but never past 80% of the available
// screen height — beyond that the scroll area takes over and
// Save/Cancel stay pinned outside it.
void SettingsDialog::capHeight() {
    adjustSize();
    if (QScreen *s = screen()) {
        int maxH = int(s->availableGeometry().height() * 0.8);
        if (height() > maxH)
            resize(width(), maxH);
    }
}

void SettingsDialog::onSourceChanged() {
    bool local = m_srcLocal->isChecked();
    m_settings.librarySource = local ? "local" : "smb";
    m_localDir->setEnabled(local);
}

void SettingsDialog::onBrowse() {
    QString dir = QFileDialog::getExistingDirectory(this, "Pick your Calibre library folder");
    if (!dir.isEmpty())
        m_localDir->setText(dir);
}

void SettingsDialog::collectSmb() {
    m_settings.librarySource = m_srcLocal->isChecked() ? "local" : "smb";
    m_settings.localDir = m_localDir->text();
    if (m_settings.servers.isEmpty())
        m_settings.servers.append(SmbServer());
    SmbServer &srv = m_settings.servers.first();
    srv.host = m_server->text();
    if (srv.shares.isEmpty())
        srv.shares.append(SmbShare());
    srv.shares.first().name = m_share->text();
    srv.shares.first().calibrePath = m_calibre->text();
    srv.user = m_user->text();
    srv.domain = m_domain->text();
    m_settings.setSmbPassword(srv.host, m_pass->text());
}

void SettingsDialog::runProbe(QLabel *pill) {
    QString cfg = m_settings.libraryConfigJson();
    QFutureWatcher<QPair<bool, QString>> *w = new QFutureWatcher<QPair<bool, QString>>(this);
    connect(w, &QFutureWatcher<QPair<bool, QString>>::finished, this, [pill, w]() {
        auto r = w->result();
        w->deleteLater();
        stylePass(pill, r.first ? "Pass" : "Fail", r.first);
    });
    w->setFuture(QtConcurrent::run([cfg]() { return probeLibrary(cfg); }));
}

void SettingsDialog::onTestLocal() {
    m_settings.librarySource = "local";
    m_settings.localDir = m_localDir->text();
    runProbe(m_localPass);
}

void SettingsDialog::onTestSmb() {
    collectSmb();
    runProbe(m_smbPass);
}

void SettingsDialog::onTestDb() {
    collectSmb();
    runProbe(m_dbPass);
}

void SettingsDialog::accept() {
    collectSmb();
    QDialog::accept();
}

void SettingsDialog::onShowPass(bool show) {
    m_pass->setEchoMode(show ? QLineEdit::Normal : QLineEdit::Password);
    if (QPushButton *b = qobject_cast<QPushButton *>(sender()))
        b->setText(show ? "Hide" : "Show");
}

void SettingsDialog::onKoboAdd() {
    QString ip = m_newKobo->text().trimmed();
    if (ip.isEmpty() || m_settings.koboIps.contains(ip))
        return;
    m_settings.koboIps.append(ip);
    if (m_settings.koboIp.trimmed().isEmpty()) {
        // First/only IP becomes the default (WPF parity) — a single
        // configured Kobo never needs starring.
        m_settings.koboIp = ip;
        m_koboStatus->setText(QString("Added %1 (default)").arg(ip));
    } else {
        m_koboStatus->setText(QString("Added %1").arg(ip));
    }
    m_newKobo->clear();
    refreshKoboRows();
}

void SettingsDialog::refreshKoboRows() {
    QLayoutItem *child;
    while ((child = m_koboRows->takeAt(0)) != nullptr) {
        delete child->widget();
        delete child;
    }
    for (const QString &ip : m_settings.koboIps) {
        QHBoxLayout *row = new QHBoxLayout();
        bool isDefault = (m_settings.koboIp == ip);
        QPushButton *star = new QPushButton(isDefault ? "★" : "☆", this);
        // Golden default, mirroring WPF: a monochrome ★ reads as
        // unchanged (the reported "star doesn't change state").
        if (isDefault)
            star->setStyleSheet("color: goldenrod; font-weight: bold; font-size: 14pt;");
        connect(star, &QPushButton::clicked, this, [this, ip]() {
            m_settings.koboIp = ip;
            m_koboStatus->setText(QString("Default Kobo: %1").arg(ip));
            refreshKoboRows();
        });
        QLineEdit *ipEdit = new QLineEdit(ip, this);
        ipEdit->setFont(QFont("Consolas"));
        ipEdit->setFixedWidth(150);
        ipEdit->setPlaceholderText("IP address");
        connect(ipEdit, &QLineEdit::editingFinished, this, [this, ip, ipEdit]() {
            QString next = ipEdit->text().trimmed();
            if (!next.isEmpty() && next != ip)
                m_settings.renameKobo(ip, next);
            refreshKoboRows();
        });
        row->addWidget(star);
        row->addWidget(ipEdit);
        row->addWidget(new QLabel("Password", this));
        bool revealed = m_revealed.contains(ip);
        QLineEdit *pwEdit = new QLineEdit(this);
        pwEdit->setFixedWidth(100);
        pwEdit->setPlaceholderText("optional");
        if (!revealed)
            pwEdit->setEchoMode(QLineEdit::Password);
        pwEdit->setText(m_settings.koboPassword(ip));
        connect(pwEdit, &QLineEdit::textChanged, this, [this, ip](const QString &t) {
            m_settings.setKoboPassword(ip, t);
        });
        QPushButton *showBtn = new QPushButton(revealed ? "Hide" : "Show", this);
        connect(showBtn, &QPushButton::clicked, this, [this, ip, showBtn]() {
            if (m_revealed.contains(ip))
                m_revealed.remove(ip);
            else
                m_revealed.insert(ip);
            refreshKoboRows();
        });
        QLabel *pill = new QLabel(this);
        fixPillWidth(pill);
        QPushButton *testBtn = new QPushButton("Test", this);
        connect(testBtn, &QPushButton::clicked, this, [this, ip, pill]() {
            pill->setText("…");
            QFutureWatcher<QString> *w = new QFutureWatcher<QString>(this);
            connect(w, &QFutureWatcher<QString>::finished, this, [pill, w]() {
                QString out = w->result();
                w->deleteLater();
                bool pass = out.trimmed() == "ok";
                pill->setText(pass ? "Pass" : "Fail");
                pill->setStyleSheet(QString("font-weight: bold; color: %1")
                                        .arg(pass ? "green" : "red"));
            });
            QString pw = m_settings.koboPassword(ip);
            w->setFuture(QtConcurrent::run([ip, pw]() {
                QJsonObject cfg;
                cfg.insert("ip", ip);
                cfg.insert("cmd", "echo ok");
                cfg.insert("password", pw);
                cfg.insert("timeout_secs", 6);
                QByteArray raw = QJsonDocument(cfg).toJson(QJsonDocument::Compact);
                QJsonValue payload;
                QString err;
                if (!ffiOk(catalog_kobo_ssh(raw.constData()), payload, err))
                    return QString("fail: ") + err;
                return payload.toObject().value("output").toString();
            }));
        });
        QPushButton *trash = new QPushButton("🗑", this);
        connect(trash, &QPushButton::clicked, this, [this, ip]() {
            m_settings.koboIps.removeAll(ip);
            m_settings.setKoboPassword(ip, "");
            if (m_settings.koboIp == ip)
                m_settings.koboIp = "";
            refreshKoboRows();
        });
        row->addWidget(pwEdit);
        row->addWidget(showBtn);
        row->addWidget(testBtn);
        row->addWidget(pill);
        row->addWidget(trash);
        row->addStretch();
        m_koboRows->addLayout(row);
    }
    capHeight();
}

void SettingsDialog::onThemeChanged(int i) {
    m_settings.theme = i;
}
