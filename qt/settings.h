#pragma once
// App settings: same %APPDATA%/com.jocala.Catalog/settings.json the WPF
// app uses (identical keys), so the Qt build migrates seamlessly.
// BrowseMode/SortOrder/ViewMode are Qt-only additions for toolbar state;
// unknown keys are ignored by JSON readers that don't know them.
#include <QByteArray>
#include <QJsonObject>
#include <QString>
#include <QStringList>

struct SmbShare {
    QString name;
    QString calibrePath;
};

struct SmbServer {
    QString label;
    QString host;
    int port = 445;
    QString user;
    QString domain;
    QList<SmbShare> shares;
};

struct AppSettings {
    QString librarySource = "smb"; // "smb" | "local"
    QString localDir;
    QList<SmbServer> servers;
    QJsonObject passwords;   // host -> smb password
    QString koboIp;          // default Kobo
    QStringList koboIps;
    QJsonObject koboPasswords; // ip -> ssh password
    bool koboHandoffPromptDone = false; // stacking-fix offer, asked once
    int theme = 0;           // 0 system, 1 light, 2 dark
    int browseMode = 0;      // toolbar browse: 0 Books, 1 Author, 2 Series, 3 Tags
    int sortOrder = 0;       // toolbar sort: CatalogStore::Sort enum value (0-4)
    QString viewMode = "grid"; // toolbar view: "grid" | "list"
    bool checkForUpdates = true; // startup update check (opt-out in Settings)
    bool diagLogging = false;    // diagnostic load timings in errors.log ([diag])
    QByteArray windowGeometry; // Qt saveGeometry blob (position + height;
                               // width is fixed by the shell, empty = default)

    QString smbPassword(const QString &host) const;
    void setSmbPassword(const QString &host, const QString &pw);
    QString koboPassword(const QString &ip) const;
    void setKoboPassword(const QString &ip, const QString &pw);
    void renameKobo(const QString &oldIp, const QString &newIp);
    const SmbServer *primaryServer() const;

    // A library source is configured when the selected source has an
    // address: first SMB server host, or a local folder. Otherwise this
    // is a fresh start and the empty page stays silent (no popup).
    bool hasSource() const;

    // FFI library-config JSON for the primary server / local dir.
    // fresh=true bypasses the FFI metadata.db cache (Reload semantics).
    // diag=true asks the engine to stash per-fetch timings for the
    // [diag] line (Settings → Diagnostic logging; off = zero extra work).
    QString libraryConfigJson(bool fresh = false, bool diag = false) const;

    static QString settingsPath();
    static AppSettings load();
    static bool save(const AppSettings &s);
};
