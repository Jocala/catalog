#include "searchdialog.h"
#include "ffi.h"
#include "ffijson.h"
#include "settings.h"
#include <QCheckBox>
#include <QComboBox>
#include <QDialogButtonBox>
#include <QFormLayout>
#include <QFutureWatcher>
#include <QJsonArray>
#include <QJsonDocument>
#include <QLineEdit>
#include <QMessageBox>
#include <QPushButton>
#include <QtConcurrent>
#include <QVBoxLayout>

SearchDialog::SearchDialog(const AppSettings &settings, QWidget *parent)
    : QDialog(parent), m_cfg(settings.libraryConfigJson()) {
    setWindowTitle("Search Books");
    setMinimumWidth(440);
    QVBoxLayout *outer = new QVBoxLayout(this);
    QFormLayout *form = new QFormLayout();
    m_query = new QLineEdit(this);
    m_query->setPlaceholderText("Search all fields (e.g. james bond, author:asimov, tag:espionage)");
    form->addRow("General", m_query);
    m_title = new QLineEdit(this);
    form->addRow("Title", m_title);
    m_author = new QLineEdit(this);
    form->addRow("Author", m_author);
    m_series = new QComboBox(this);
    m_series->setEditable(true);
    form->addRow("Series", m_series);
    m_tags = new QComboBox(this);
    m_tags->setEditable(true);
    form->addRow("Tag", m_tags);
    m_expand = new QCheckBox("Expand tag to series", this);
    m_expand->setChecked(true);
    form->addRow("", m_expand);
    outer->addLayout(form);
    QDialogButtonBox *buttons = new QDialogButtonBox(
        QDialogButtonBox::Ok | QDialogButtonBox::Cancel, this);
    buttons->button(QDialogButtonBox::Ok)->setText("Search");
    connect(buttons, &QDialogButtonBox::accepted, this, [this]() {
        if (m_query->text().trimmed().isEmpty() && m_title->text().trimmed().isEmpty()
            && m_author->text().trimmed().isEmpty()
            && m_series->currentText().trimmed().isEmpty()
            && m_tags->currentText().trimmed().isEmpty()) {
            QMessageBox::information(this, "Search", "Fill at least one field.");
            return;
        }
        accept();
    });
    connect(buttons, &QDialogButtonBox::rejected, this, &SearchDialog::reject);
    outer->addWidget(buttons);
    loadLists();
}

QJsonObject SearchDialog::params() const {
    QJsonObject p;
    p.insert("query", m_query->text());
    p.insert("title", m_title->text());
    p.insert("author", m_author->text());
    p.insert("series", m_series->currentText());
    p.insert("tag", m_tags->currentText());
    p.insert("expand_tag", m_expand->isChecked());
    return p;
}

void SearchDialog::loadLists() {
    QString cfg = m_cfg;
    QFutureWatcher<QPair<QStringList, QStringList>> *w =
        new QFutureWatcher<QPair<QStringList, QStringList>>(this);
    connect(w, &QFutureWatcher<QPair<QStringList, QStringList>>::finished, this,
            [this, w]() {
                auto r = w->result();
                w->deleteLater();
                m_series->addItems(r.first);
                m_tags->addItems(r.second);
                // Leave both blank: without this the combos default to
                // index 0 and the dialog looks pre-filled with the first
                // series/tag in the library.
                m_series->setCurrentIndex(-1);
                m_tags->setCurrentIndex(-1);
            });
    w->setFuture(QtConcurrent::run([cfg]() {
        QPair<QStringList, QStringList> out;
        QByteArray cfgB = cfg.toUtf8();
        QJsonValue payload;
        QString err;
        if (ffiOk(catalog_browse(cfgB.constData(), "series", false, false), payload, err)) {
            for (const QJsonValue &e : payload.toArray())
                out.first.append(e.toObject().value("name").toString());
        }
        if (ffiOk(catalog_browse(cfgB.constData(), "tags", false, false), payload, err)) {
            for (const QJsonValue &e : payload.toArray())
                out.second.append(e.toObject().value("name").toString());
        }
        return out;
    }));
}
