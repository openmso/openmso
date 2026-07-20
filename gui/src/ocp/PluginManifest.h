#pragma once

#include <QString>
#include <QStringList>

namespace openmso::ocp {

// Resolved plugin manifest: a name, a launch argv (with {python}
// expanded and relative paths resolved against the plugin dir), and
// the originating directory. Mirrors python/openmso/client.py's
// find_plugin() output.
struct PluginManifest {
    QString name;
    QString description;
    QStringList argv;     // resolved, ready for QProcess::start
    QString pluginDir;
};

// Walk `<pluginsDir>/<name>/plugin.json` and return one manifest per
// plugin found. Plugins whose manifest is unreadable or missing the
// `run` key are silently skipped (a warning is sent to qWarning()).
// `pluginsDir` is typically `<repo>/plugins` in developer mode or
// `<install_prefix>/lib/openmso/plugins` for a bundled install.
//
// {python} expansion: OPENMSO_PYTHON env var if set, else "python3"
// on Unix / "python.exe" on Windows (bundled interpreter).
QList<PluginManifest> findPlugins(const QString &pluginsDir);

// Resolve a single plugin by name. Returns an empty PluginManifest
// (empty name) if not found.
PluginManifest findPlugin(const QString &pluginsDir, const QString &name);

// Resolve {python} in argv[0..] according to env/platform rules.
QStringList expandPython(const QStringList &argv, const QString &pluginDir);

} // namespace openmso::ocp
