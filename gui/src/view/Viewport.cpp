#include "Viewport.h"

#include "Trace.h"

#include <QApplication>
#include <QMouseEvent>
#include <QPainter>
#include <QPaintEvent>
#include <QPalette>
#include <QWheelEvent>
#include <QtMath>
#include <cmath>

namespace openmso::view {

Viewport::Viewport(QWidget *parent) : QWidget(parent)
{
    setFocusPolicy(Qt::StrongFocus);
    setMouseTracking(true);
    setMinimumSize(400, 200);
    // Slightly darker than the window background so the viewport
    // stands out.
    setBackgroundRole(QPalette::Base);
    setAutoFillBackground(true);
}

void Viewport::setViewState(ViewState *st)
{
    if (state_ == st) return;
    if (state_) disconnect(state_, nullptr, this, nullptr);
    state_ = st;
    if (state_)
        connect(state_, &ViewState::changed, this,
                qOverload<>(&QWidget::update));
    update();
}

void Viewport::setTraces(const QList<Trace *> &traces)
{
    traces_.clear();
    for (auto *t : traces) {
        if (!t) continue;
        t->setParent(this);
        traces_.append(t);
    }
    update();
}

void Viewport::clearTraces()
{
    traces_.clear();
    update();
}

int Viewport::contentHeight() const
{
    int h = 0;
    for (const auto &t : traces_)
        if (t) h += t->height();
    return h;
}

void Viewport::paintEvent(QPaintEvent *)
{
    QPainter p(this);
    p.setRenderHint(QPainter::Antialiasing, true);

    const QRect r = rect();
    // Background.
    p.fillRect(r, palette().base());
    if (!state_) return;

    // Stack traces top to bottom, shifted by the vertical scroll offset.
    int y = -state_->yOffset();
    for (const auto &t : traces_) {
        if (!t) continue;
        QRect row(r.left(), y, r.width(), t->height());
        if (row.intersects(r)) {
            p.save();
            p.setClipRect(row);
            t->paintMid(p, row, *state_);
            t->paintFore(p, row, *state_);
            p.restore();
        }
        y += t->height();
    }

    // Cursor overlays.
    if (state_->cursorsVisible()) {
        p.setPen(QPen(palette().text().color(), 1, Qt::DashLine));
        if (state_->cursorA() >= 0) {
            int x = int(state_->timeToX(state_->cursorA()));
            p.drawLine(x, 0, x, r.height());
        }
        if (state_->cursorB() >= 0) {
            int x = int(state_->timeToX(state_->cursorB()));
            p.drawLine(x, 0, x, r.height());
        }
    }

    // Trigger marker.
    if (!std::isnan(state_->triggerPos())) {
        int x = int(state_->timeToX(state_->triggerPos()));
        p.setPen(QPen(QColor(255, 200, 0), 1));
        p.drawLine(x, 0, x, r.height());
    }
}

void Viewport::wheelEvent(QWheelEvent *e)
{
    if (!state_) { e->ignore(); return; }

    const QPoint ad = e->angleDelta();

    if (e->modifiers() & Qt::ControlModifier) {
        // Ctrl+wheel: zoom the time axis around the pointer. Scale the
        // factor to the wheel delta so a trackpad's many small events add
        // up smoothly instead of doubling per event (was way too twitchy).
        if (ad.y() != 0) {
            const double factor = std::pow(1.0015, -ad.y());
            zoomAt(double(e->position().x()), factor);
        }
        e->accept();
        return;
    }

    // Horizontal side-scroll: a trackpad's x delta, or Shift+wheel which
    // maps the vertical wheel onto the x axis. A rightward gesture
    // (positive delta) reveals later time, so pan the offset up — hence
    // the negated delta feeding panPixels (which moves content left for a
    // positive pixel argument).
    int dx = ad.x();
    if (dx == 0 && (e->modifiers() & Qt::ShiftModifier))
        dx = ad.y();
    if (dx != 0)
        panPixels(-dx / 2.0);

    // Vertical scroll through the trace stack (plain wheel).
    if (dx == 0 && ad.y() != 0) {
        const int maxY = std::max(0, contentHeight() - height());
        int y = state_->yOffset() - ad.y() / 2;
        y = std::max(0, std::min(maxY, y));
        state_->setYOffset(y);
    }
    e->accept();
}

void Viewport::zoom(double factor)
{
    zoomAt(width() / 2.0, factor);
}

void Viewport::toggleCursors()
{
    if (!state_) return;
    const bool vis = !state_->cursorsVisible();
    if (vis && state_->cursorA() < 0)
        state_->setCursors(state_->xToTime(width() * 0.3),
                           state_->xToTime(width() * 0.7));
    state_->setCursorsVisible(vis);
}

void Viewport::zoomAt(double x, double factor)
{
    if (!state_) return;
    // Keep the sample under the pointer fixed.
    const double tAtX = state_->xToTime(x);
    const double newScale = state_->scale() * factor;
    state_->setScaleOffset(newScale, tAtX - x * newScale);
}

void Viewport::panPixels(double dxPixels)
{
    if (!state_) return;
    state_->setOffset(state_->offset() + dxPixels * state_->scale());
}

void Viewport::mousePressEvent(QMouseEvent *e)
{
    if (!state_) return;
    if (e->button() == Qt::LeftButton) {
        // Left-drag lays down a cursor selection (Audacity-style). A bare
        // click collapses A and B to a single point. Panning is done with
        // the scrollbar / Shift+wheel, not by dragging the waveform.
        selecting_ = true;
        selAnchor_ = state_->xToTime(e->position().x());
        state_->setCursorsVisible(true);
        state_->setCursors(selAnchor_, selAnchor_);
    }
}

void Viewport::mouseMoveEvent(QMouseEvent *e)
{
    if (!state_ || !selecting_) return;
    const double t = state_->xToTime(e->position().x());
    // Keep A <= B so the Δt readout and shading stay tidy regardless of
    // drag direction.
    state_->setCursors(std::min(selAnchor_, t), std::max(selAnchor_, t));
}

void Viewport::mouseReleaseEvent(QMouseEvent *e)
{
    if (e->button() == Qt::LeftButton)
        selecting_ = false;
}

void Viewport::keyPressEvent(QKeyEvent *e)
{
    if (!state_) { QWidget::keyPressEvent(e); return; }
    const double panStep = width() * 0.1;
    switch (e->key()) {
    case Qt::Key_Plus:
    case Qt::Key_Equal:
        zoomAt(width() / 2.0, 0.8);
        break;
    case Qt::Key_Minus:
        zoomAt(width() / 2.0, 1.25);
        break;
    case Qt::Key_Left:
        panPixels(-panStep);
        break;
    case Qt::Key_Right:
        panPixels(panStep);
        break;
    case Qt::Key_Home:
        // Jump to trigger.
        if (!std::isnan(state_->triggerPos()))
            state_->setOffset(state_->triggerPos() - width() / 2.0 * state_->scale());
        break;
    case Qt::Key_C:
        toggleCursors();
        break;
    default:
        QWidget::keyPressEvent(e);
    }
}

} // namespace openmso::view
