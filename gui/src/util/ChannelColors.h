#pragma once

#include <QColor>
#include <QPalette>

namespace openmso::util {

// Theme selector for palette lookups. The GUI follows the platform
// light/dark mode; colors are luminance-tuned per theme so they stay
// legible on either background. Dark is the primary target.
enum class Theme { Light, Dark };

// Detect the effective theme from a widget palette (window lightness).
Theme themeFor(const QPalette &palette);

// The dark palette the app installs when the desktop asks for dark and the
// platform theme has not supplied one of its own.
QPalette darkPalette();

// Logic channel color by channel index (the "D" number). Follows the
// decimal resistor color code starting at black: 0=black, 1=brown,
// 2=red, 3=orange, 4=yellow, 5=green, 6=blue, 7=violet. The 8-color set
// repeats for D8..D15 (index is taken modulo 8), matching the Saleae
// Logic 16 wire harness. Black is lifted to a light neutral on dark
// backgrounds so D0 stays visible; white is reserved for ground and is
// never a channel color.
QColor logicColor(int channelIndex, Theme theme);

// Analog channel color by channel index. Follows the oscilloscope
// convention shared by PulseView/Keysight/Siglent: 0=yellow, 1=magenta,
// 2=cyan, 3=green. Repeats (index modulo 4) beyond four channels.
QColor analogColor(int channelIndex, Theme theme);

} // namespace openmso::util
