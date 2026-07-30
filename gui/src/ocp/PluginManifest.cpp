#include "PluginManifest.h"

#include <QDir>
#include <QFileInfo>
#include <QProcessEnvironment>

namespace openmso::ocp {

namespace {

PluginManifest fromDir(const QDir &dir)
{
    PluginManifest out;
    try {
        const auto message =
            ::openmso::manifest::load(dir.absolutePath().toStdString());
        const auto argv = ::openmso::manifest::resolveArgv(
            message, dir.absolutePath().toStdString(),
            pythonInterpreter().toStdString());

        out.message = message;
        out.name = QString::fromStdString(message.name());
        out.description = QString::fromStdString(message.description());
        out.pluginDir = dir.absolutePath();
        for (const auto &token : argv)
            out.argv.append(QString::fromStdString(token));
        for (const auto &scheme : message.url_schemes())
            out.urlSchemes.append(QString::fromStdString(scheme));
    } catch (const ::openmso::Error &e) {
        qWarning("plugin manifest in %s: %s",
                 qUtf8Printable(dir.absolutePath()), e.what());
        return PluginManifest();
    }

    if (out.argv.isEmpty()) {
        qWarning("plugin manifest in %s has no run argv",
                 qUtf8Printable(dir.absolutePath()));
        return PluginManifest();
    }
    return out;
}

} // namespace

QString pythonInterpreter()
{
    const auto env = QProcessEnvironment::systemEnvironment();
    const QString override = env.value(QStringLiteral("OPENMSO_PYTHON"));
    if (!override.isEmpty())
        return override;
#ifdef Q_OS_WIN
    return QStringLiteral("python.exe");
#else
    return QStringLiteral("python3");
#endif
}

QList<PluginManifest> findPlugins(const QString &pluginsDir)
{
    QList<PluginManifest> out;
    QDir root(pluginsDir);
    if (!root.exists())
        return out;

    const auto entries =
        root.entryList(QDir::Dirs | QDir::NoDotAndDotDot, QDir::Name);
    for (const QString &entry : entries) {
        QDir dir(root.filePath(entry));
        if (!QFileInfo::exists(dir.filePath(
                QString::fromLatin1(::openmso::manifest::FILENAME))))
            continue;
        PluginManifest m = fromDir(dir);
        if (!m.isNull())
            out.append(m);
    }
    return out;
}

PluginManifest findPlugin(const QString &pluginsDir, const QString &name)
{
    for (const auto &m : findPlugins(pluginsDir)) {
        if (m.name == name)
            return m;
    }
    return PluginManifest();
}

QStringList candidateDeviceUrls(const PluginManifest &manifest)
{
    QStringList urls;
    if (manifest.urlSchemes.contains(QStringLiteral("demo")))
        urls << QStringLiteral("demo://0");

    if (manifest.urlSchemes.contains(QStringLiteral("usb"))) {
        for (const auto &usbId : manifest.message.usb_ids())
            urls << QStringLiteral("usb://") + QString::fromStdString(usbId.id());
    }
    return urls;
}

} // namespace openmso::ocp
