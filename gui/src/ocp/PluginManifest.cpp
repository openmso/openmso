#include "PluginManifest.h"

#include <QDir>
#include <QDirIterator>
#include <QFile>
#include <QFileInfo>
#include <QJsonArray>
#include <QJsonDocument>
#include <QJsonObject>
#include <QProcessEnvironment>
#include <QStandardPaths>

namespace openmso::ocp {

namespace {

QString defaultPython()
{
    const auto env = QProcessEnvironment::systemEnvironment();
    if (env.contains(QStringLiteral("OPENMSO_PYTHON")))
        return env.value(QStringLiteral("OPENMSO_PYTHON"));
#ifdef Q_OS_WIN
    return QStringLiteral("python.exe");
#else
    return QStringLiteral("python3");
#endif
}

// Apply the same "relative paths resolve against the plugin dir" rule
// as python/openmso/client.py:find_plugin. A token is considered a
// path if it contains a separator or ends with .py; plain flags pass
// through unchanged.
QString resolveArg(const QString &token, const QString &pluginDir)
{
    if (token == QStringLiteral("{python}"))
        return defaultPython();
    if (token.contains(QDir::separator()) || token.endsWith(QStringLiteral(".py"))
        || token.contains('/')) {
        QFileInfo fi(token);
        if (fi.isAbsolute())
            return QDir::cleanPath(token);
        return QDir::cleanPath(QDir(pluginDir).absoluteFilePath(token));
    }
    return token;
}

} // namespace

QStringList expandPython(const QStringList &argv, const QString &pluginDir)
{
    QStringList out;
    out.reserve(argv.size());
    for (const auto &a : argv)
        out.append(resolveArg(a, pluginDir));
    return out;
}

PluginManifest findPlugin(const QString &pluginsDir, const QString &name)
{
    PluginManifest m;
    const QString dir = QDir(pluginsDir).absoluteFilePath(name);
    const QString manifestPath = QDir(dir).absoluteFilePath(QStringLiteral("plugin.json"));
    QFile f(manifestPath);
    if (!f.open(QIODevice::ReadOnly)) {
        qWarning("findPlugin: cannot open %s", qPrintable(manifestPath));
        return m;
    }
    QJsonParseError err;
    auto doc = QJsonDocument::fromJson(f.readAll(), &err);
    if (err.error != QJsonParseError::NoError || !doc.isObject()) {
        qWarning("findPlugin: bad JSON in %s: %s",
                 qPrintable(manifestPath), qPrintable(err.errorString()));
        return m;
    }
    const auto obj = doc.object();
    const auto run = obj.value(QStringLiteral("run")).toArray();
    if (run.isEmpty()) {
        qWarning("findPlugin: %s has no \"run\" array", qPrintable(manifestPath));
        return m;
    }

    m.name = obj.value(QStringLiteral("name")).toString(name);
    m.description = obj.value(QStringLiteral("description")).toString();
    m.pluginDir = dir;
    QStringList raw;
    raw.reserve(run.size());
    for (const auto &v : run)
        raw.append(v.toString());
    m.argv = expandPython(raw, dir);
    return m;
}

QList<PluginManifest> findPlugins(const QString &pluginsDir)
{
    QList<PluginManifest> out;
    QDirIterator it(pluginsDir, QDir::Dirs | QDir::NoDotAndDotDot);
    while (it.hasNext()) {
        it.next();
        const QString name = it.fileName();
        auto m = findPlugin(pluginsDir, name);
        if (!m.name.isEmpty() && !m.argv.isEmpty())
            out.append(std::move(m));
    }
    std::sort(out.begin(), out.end(),
              [](const PluginManifest &a, const PluginManifest &b) {
                  return a.name.compare(b.name, Qt::CaseInsensitive) < 0;
              });
    return out;
}

} // namespace openmso::ocp
