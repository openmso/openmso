#include "Ruler.h"

#include "util/TimeFormat.h"

#include <QFontMetrics>
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

void Ruler::setViewState(ViewState *st)
{
    if (st_ == st) return;
    if (st_) disconnect(st_, nullptr, this, nullptr);
    st_ = st;
    if (st_)
        connect(st_, &ViewState::changed, this,
                qOverload<>(&QWidget::update));
    update();
}

void Ruler::paintEvent(QPaintEvent *)
{
    QPainter p(this);
    const QRect r = rect();
    p.fillRect(r, palette().window());
    if (!st_) return;

    p.setPen(palette().text().color());

    const QFontMetrics fm = p.fontMetrics();
    // Choose a tick spacing whose labels won't collide: target at least
    // the width of a representative label plus padding.
    const double minLabelPx = fm.horizontalAdvance(QStringLiteral("000.000 ms")) + 16;
    const double spacing = util::niceTickStep(st_->scale() * minLabelPx);
    const int decimals = util::decimalsForStep(spacing, st_->offset() + r.width() * st_->scale());

    const double tLeft = st_->offset();
    const double tRight = st_->offset() + r.width() * st_->scale();
    double t = std::floor(tLeft / spacing) * spacing;

    int lastLabelRight = -10000;
    while (t <= tRight) {
        const int x = int(st_->timeToX(t));
        p.drawLine(x, r.bottom() - 6, x, r.bottom());
        // Minor tick at the half step.
        const int xh = int(st_->timeToX(t + spacing / 2.0));
        p.drawLine(xh, r.bottom() - 3, xh, r.bottom());
        // Only draw the label if it clears the previous one.
        const QString label = util::formatTime(t, decimals);
        const int lw = fm.horizontalAdvance(label);
        if (x + 3 > lastLabelRight + 6) {
            p.drawText(x + 3, r.top() + 14, label);
            lastLabelRight = x + 3 + lw;
        }
        t += spacing;
    }

    // Cursor labels at top.
    if (st_->cursorsVisible()) {
        p.setPen(QPen(QColor(180, 180, 220), 1, Qt::DashLine));
        if (st_->cursorA() >= 0) {
            int x = int(st_->timeToX(st_->cursorA()));
            p.drawLine(x, r.top(), x, r.bottom());
            p.drawText(x + 3, r.top() + 12, "A");
        }
        if (st_->cursorB() >= 0) {
            int x = int(st_->timeToX(st_->cursorB()));
            p.drawLine(x, r.top(), x, r.bottom());
            p.drawText(x + 3, r.top() + 12, "B");
        }
    }
}

} // namespace openmso::view
