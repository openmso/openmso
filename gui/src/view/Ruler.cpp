#include "Ruler.h"

#include <QPainter>
#include <QPaintEvent>
#include <QPalette>
#include <cmath>

namespace openmso::view {

Ruler::Ruler(QWidget *parent) : QWidget(parent)
{
    setAutoFillBackground(true);
    setBackgroundRole(QPalette::Window);
}

namespace {

// Pick a "nice" tick spacing: 1/2/5 × 10^n seconds, such that the
// spacing in pixels is between 60 and 120.
double niceTickSpacing(double scale)
{
    // seconds per 80px target
    double target = scale * 80.0;
    double mag = std::pow(10.0, std::floor(std::log10(target)));
    double norm = target / mag;
    double step;
    if (norm < 1.5)      step = 1.0;
    else if (norm < 3.5) step = 2.0;
    else if (norm < 7.5) step = 5.0;
    else                 step = 10.0;
    return step * mag;
}

QString formatTime(double t)
{
    double a = std::abs(t);
    if (a >= 1.0)      return QStringLiteral("%1 s").arg(t, 0, 'f', 3);
    if (a >= 1e-3)     return QStringLiteral("%1 ms").arg(t * 1e3, 0, 'f', 3);
    if (a >= 1e-6)     return QStringLiteral("%1 µs").arg(t * 1e6, 0, 'f', 3);
    return QStringLiteral("%1 ns").arg(t * 1e9, 0, 'f', 1);
}

} // namespace

void Ruler::paintEvent(QPaintEvent *)
{
    QPainter p(this);
    const QRect r = rect();
    p.fillRect(r, palette().window());

    p.setPen(palette().text().color());

    const double spacing = niceTickSpacing(st_.scale);
    const double tLeft = st_.offset;
    const double tRight = st_.offset + r.width() * st_.scale;
    double t = std::floor(tLeft / spacing) * spacing;

    while (t <= tRight) {
        int x = int(st_.timeToX(t));
        p.drawLine(x, r.bottom() - 6, x, r.bottom());
        p.drawText(x + 3, r.top() + 14, formatTime(t));
        t += spacing;
    }

    // Cursor labels at top.
    if (st_.cursorsVisible) {
        p.setPen(QPen(QColor(180, 180, 220), 1, Qt::DashLine));
        if (st_.cursorA >= 0) {
            int x = int(st_.timeToX(st_.cursorA));
            p.drawLine(x, r.top(), x, r.bottom());
            p.drawText(x + 3, r.top() + 12, "A");
        }
        if (st_.cursorB >= 0) {
            int x = int(st_.timeToX(st_.cursorB));
            p.drawLine(x, r.top(), x, r.bottom());
            p.drawText(x + 3, r.top() + 12, "B");
        }
        if (st_.cursorA >= 0 && st_.cursorB >= 0) {
            double dt = std::abs(st_.cursorB - st_.cursorA);
            QString label = QStringLiteral("Δt=%1  f=%2")
                                .arg(formatTime(dt))
                                .arg(dt > 0 ? formatTime(1.0 / dt)
                                            : QStringLiteral("∞"));
            p.setPen(palette().text().color());
            p.drawText(r.center().x() - 60, r.top() + 12, label);
        }
    }
}

} // namespace openmso::view
