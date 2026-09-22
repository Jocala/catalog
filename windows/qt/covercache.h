#pragma once
// Shell-level on-disk cover cache: book path -> JPEG bytes. Bounded by
// total bytes (default 256 MB); Put evicts oldest-first, so a 6700-book
// library can never grow this without limit. QImage decode stays with
// the caller (worker thread for fetches, UI thread for disk hits).
#include <QByteArray>
#include <QMutex>
#include <QString>

class DiskCoverCache {
public:
    explicit DiskCoverCache(const QString &rootDir,
                            qint64 maxBytes = 256LL * 1024 * 1024);
    static QString defaultRoot();
    static QString keyFor(const QString &bookPath); // sha256 hex + .jpg
    bool tryGet(const QString &bookPath, QByteArray &out);
    void put(const QString &bookPath, const QByteArray &jpeg);
    QString root() const { return m_root; }

private:
    void enforceCap();
    QString m_root;
    qint64 m_max;
    QMutex m_gate;
};
