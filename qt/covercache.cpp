#include "covercache.h"
#include <QCryptographicHash>
#include <QDir>
#include <QFile>
#include <QFileInfo>
#include <QStandardPaths>

DiskCoverCache::DiskCoverCache(const QString &rootDir, qint64 maxBytes)
    : m_root(rootDir), m_max(maxBytes) {
    QDir().mkpath(m_root);
}

QString DiskCoverCache::defaultRoot() {
    return QStandardPaths::writableLocation(QStandardPaths::CacheLocation)
        + "/catalog-covers";
}

QString DiskCoverCache::keyFor(const QString &bookPath) {
    QByteArray hash = QCryptographicHash::hash(bookPath.toUtf8(),
                                               QCryptographicHash::Sha256);
    return QString::fromLatin1(hash.toHex()) + ".jpg";
}

bool DiskCoverCache::tryGet(const QString &bookPath, QByteArray &out) {
    QMutexLocker lock(&m_gate);
    QFile f(m_root + "/" + keyFor(bookPath));
    if (!f.open(QIODevice::ReadOnly))
        return false;
    out = f.readAll();
    return !out.isEmpty();
}

void DiskCoverCache::put(const QString &bookPath, const QByteArray &jpeg) {
    if (jpeg.isEmpty())
        return;
    QMutexLocker lock(&m_gate);
    const QString key = m_root + "/" + keyFor(bookPath);
    qint64 oldSize = 0;
    if (QFileInfo fi(key); fi.exists())
        oldSize = fi.size();
    QFile f(key);
    if (!f.open(QIODevice::WriteOnly | QIODevice::Truncate))
        return;
    f.write(jpeg);
    f.close();
    // Full dir scan per cover stalls fetch workers on big libraries
    // (700+ files); track a running total and scan only when over cap.
    if (m_approxTotal < 0) {
        m_approxTotal = enforceCap(); // learns the true total, evicts if needed
    } else {
        m_approxTotal += jpeg.size() - oldSize;
    }
    if (m_approxTotal > m_max)
        m_approxTotal = enforceCap();
}

qint64 DiskCoverCache::enforceCap() {
    QDir dir(m_root, "*.jpg");
    QFileInfoList files = dir.entryInfoList(QDir::Files, QDir::Time);
    qint64 total = 0;
    for (const QFileInfo &fi : files)
        total += fi.size();
    if (total <= m_max)
        return total;
    // entryInfoList(Time) sorts newest first: evict from the end.
    for (int i = files.size() - 1; i >= 0 && total > m_max; --i) {
        total -= files[i].size();
        QFile::remove(files[i].absoluteFilePath());
    }
    return total;
}
