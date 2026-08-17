#include "ChannelColors.h"

#include <QPalette>

namespace openmso::util {

Theme themeFor(const QPalette &palette)
{
    // Compare the window background lightness; < 50% → dark theme.
    return palette.color(QPalette::Window).lightness() < 128
               ? Theme::Dark
               : Theme::Light;
}

QPalette darkPalette()
{
    // Matches docs/gui-plan/tools/dark_render.cpp, which the progress
    // screenshots were rendered against.
    const QColor window(45, 45, 50), base(30, 30, 34), text(220, 220, 225);
    const QColor disabled(120, 120, 128), highlight(38, 110, 180);

    QPalette p;
    p.setColor(QPalette::Window, window);
    p.setColor(QPalette::Base, base);
    p.setColor(QPalette::AlternateBase, window);
    p.setColor(QPalette::Button, window);
    p.setColor(QPalette::ToolTipBase, base);
    p.setColor(QPalette::WindowText, text);
    p.setColor(QPalette::Text, text);
    p.setColor(QPalette::ButtonText, text);
    p.setColor(QPalette::ToolTipText, text);
    p.setColor(QPalette::Highlight, highlight);
    p.setColor(QPalette::HighlightedText, text);
    for (auto role : {QPalette::WindowText, QPalette::Text, QPalette::ButtonText})
        p.setColor(QPalette::Disabled, role, disabled);
    return p;
}

QColor logicColor(int channelIndex, Theme theme)
{
    // Decimal resistor code 0..7 (black, brown, red, orange, yellow,
    // green, blue, violet), tuned per theme. Black is lifted to a light
    // neutral on dark backgrounds. Two luminance points per hue.
    static const QColor dark[8] = {
        QColor("#DCDCDC"),  // 0 black  (lifted for visibility on dark)
        QColor("#C08457"),  // 1 brown
        QColor("#E24444"),  // 2 red
        QColor("#E88A2A"),  // 3 orange
        QColor("#E5CE2A"),  // 4 yellow
        QColor("#5FC15F"),  // 5 green
        QColor("#4E96E6"),  // 6 blue
        QColor("#B072D6"),  // 7 violet
    };
    static const QColor light[8] = {
        QColor("#1A1A1A"),  // 0 black
        QColor("#8B5A2B"),  // 1 brown
        QColor("#C62828"),  // 2 red
        QColor("#E65100"),  // 3 orange
        QColor("#9A8500"),  // 4 yellow (darkened; bright yellow is
                            //   invisible on light backgrounds)
        QColor("#2E7D32"),  // 5 green
        QColor("#1565C0"),  // 6 blue
        QColor("#7B1FA2"),  // 7 violet
    };
    const int i = ((channelIndex % 8) + 8) % 8;
    return theme == Theme::Dark ? dark[i] : light[i];
}

QColor analogColor(int channelIndex, Theme theme)
{
    // Oscilloscope order: yellow, magenta, cyan, green.
    static const QColor dark[4] = {
        QColor("#E5CE2A"),  // yellow
        QColor("#E24AB0"),  // magenta
        QColor("#35C4D4"),  // cyan
        QColor("#5FC15F"),  // green
    };
    static const QColor light[4] = {
        QColor("#9A8500"),  // yellow (darkened)
        QColor("#C2185B"),  // magenta
        QColor("#0097A7"),  // cyan
        QColor("#2E7D32"),  // green
    };
    const int i = ((channelIndex % 4) + 4) % 4;
    return theme == Theme::Dark ? dark[i] : light[i];
}

} // namespace openmso::util
