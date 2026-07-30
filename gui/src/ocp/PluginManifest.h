#pragma once

#include <openmso/manifest.h>

#include <QList>
#include <QString>
#include <QStringList>

namespace openmso::ocp {

// A plugin.json found on disk, with its launch argv already resolved.
struct PluginManifest {
    QString name;
    QString description;
    QStringList argv;
    QString pluginDir;
    QStringList urlSchemes;

    // The parsed message, for the fields the GUI reads case by case
    // (palette, usbIds).
    ::openmso::pb::PluginManifest message;

    bool isNull() const { return name.isEmpty(); }
};

// Reads `<pluginsDir>/<name>/plugin.json`. A manifest that fails to parse is
// skipped with a qWarning() rather than aborting the scan.
QList<PluginManifest> findPlugins(const QString &pluginsDir);

// Null manifest if there is no such plugin.
PluginManifest findPlugin(const QString &pluginsDir, const QString &name);

// Device URLs worth trying for this plugin, in declaration order.
//
// Stands in for the frontend USB enumeration of the OCP v1 design: until the
// GUI can see the bus itself it cannot tell which of a manifest's usbIds is
// plugged in, so it offers each in turn and keeps the one that answers Hello.
QStringList candidateDeviceUrls(const PluginManifest &manifest);

// OPENMSO_PYTHON if set, else python3 (python.exe on Windows).
QString pythonInterpreter();

} // namespace openmso::ocp
