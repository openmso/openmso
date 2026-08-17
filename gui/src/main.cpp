#include "ui/MainWindow.h"
#include "util/ChannelColors.h"

#include <QApplication>
#include <QDBusConnection>
#include <QDBusInterface>
#include <QDBusReply>
#include <QDBusVariant>
#include <QPalette>
#include <QStyleHints>

namespace {

// Qt reports Light on desktops that are set to dark (GNOME 49 / Qt 6.8), so
// the portal setting is the authority when it disagrees:
// org.freedesktop.appearance color-scheme, 1 = prefer dark.
bool desktopPrefersDark()
{
    if (QApplication::styleHints()->colorScheme() == Qt::ColorScheme::Dark)
        return true;

    QDBusInterface settings(QStringLiteral("org.freedesktop.portal.Desktop"),
                            QStringLiteral("/org/freedesktop/portal/desktop"),
                            QStringLiteral("org.freedesktop.portal.Settings"),
                            QDBusConnection::sessionBus());
    const QDBusReply<QDBusVariant> scheme =
        settings.call(QStringLiteral("ReadOne"),
                      QStringLiteral("org.freedesktop.appearance"),
                      QStringLiteral("color-scheme"));
    return scheme.isValid() && scheme.value().variant().toUInt() == 1;
}

// A platform theme (qt6ct, KDE) that supplies a dark palette has already made
// this choice, and one supplying a light palette is an explicit choice to
// leave alone, so only an undecorated light palette is replaced.
void followColorScheme(const QPalette &platform)
{
    using namespace openmso::util;
    const bool replace = desktopPrefersDark() && themeFor(platform) != Theme::Dark;
    QApplication::setPalette(replace ? darkPalette() : platform);
}

} // namespace

int main(int argc, char *argv[])
{
    QApplication app(argc, argv);
    QApplication::setApplicationName("omso");
    QApplication::setApplicationVersion("0.1.0");
    QApplication::setOrganizationName("OpenMSO");

    // The platform's own palette, kept so a later switch back to light
    // restores it rather than a guess at what it was.
    const QPalette platform = app.palette();
    followColorScheme(platform);
    QObject::connect(app.styleHints(), &QStyleHints::colorSchemeChanged, &app,
                     [platform] { followColorScheme(platform); });

    openmso::ui::MainWindow window;
    window.show();

    return app.exec();
}
