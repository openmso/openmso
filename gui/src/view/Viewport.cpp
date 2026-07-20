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

    // Stack traces top to bottom.
    int y = 0;
    for (const auto &t : traces_) {
        if (!t) continue;
        QRect row(r.left(), y, r.width(), t->height());
        if (row.intersects(r)) {
            p.save();
            p.setClipRect(row);
            t->paintMid(p, row, state_);
            t->paintFore(p, row, state_);
            p.restore();
        }
        y += t->height();
    }

    // Cursor overlays.
    if (state_.cursorsVisible) {
        p.setPen(QPen(palette().text().color(), 1, Qt::DashLine));
        if (state_.cursorA >= 0) {
            int x = int(state_.timeToX(state_.cursorA));
            p.drawLine(x, 0, x, r.height());
        }
        if (state_.cursorB >= 0) {
            int x = int(state_.timeToX(state_.cursorB));
            p.drawLine(x, 0, x, r.height());
        }
    }

    // Trigger marker.
    if (!std::isnan(state_.triggerPos)) {
        int x = int(state_.timeToX(state_.triggerPos));
        p.setPen(QPen(QColor(255, 200, 0), 1));
        p.drawLine(x, 0, x, r.height());
    }
}

void Viewport::wheelEvent(QWheelEvent *e)
{
    const double factor = (e->angleDelta().y() > 0) ? 0.5 : 2.0;
    zoomAt(double(e->position().x()), factor);
}

void Viewport::zoomAt(double x, double factor)
{
    // Keep the sample under the cursor fixed.
    const double tAtX = state_.xToTime(x);
    state_.scale *= factor;
    // Clamp scale to a sane range.
    state_.scale = std::max(1e-12, std::min(1e3, state_.scale));
    state_.offset = tAtX - x * state_.scale;
    emit stateChanged();
    update();
}

void Viewport::mousePressEvent(QMouseEvent *e)
{
    if (e->button() == Qt::LeftButton) {
        // If near a visible cursor, drag it; else pan.
        if (state_.cursorsVisible) {
            const int xA = int(state_.timeToX(state_.cursorA));
            const int xB = int(state_.timeToX(state_.cursorB));
            if (state_.cursorA >= 0 && std::abs(e->position().x() - xA) < 5) {
                cursorDragging_ = true;
                cursorDragWhich_ = 0;
                return;
            }
            if (state_.cursorB >= 0 && std::abs(e->position().x() - xB) < 5) {
                cursorDragging_ = true;
                cursorDragWhich_ = 1;
                return;
            }
        }
        dragging_ = true;
        dragStart_ = e->pos();
        dragOffsetStart_ = state_.offset;
        setCursor(Qt::ClosedHandCursor);
    }
}

void Viewport::mouseMoveEvent(QMouseEvent *e)
{
    if (cursorDragging_) {
        double t = state_.xToTime(e->position().x());
        if (cursorDragWhich_ == 0) state_.cursorA = t;
        else                       state_.cursorB = t;
        emit cursorMoved(state_.cursorA, state_.cursorB);
        emit stateChanged();
        update();
    } else if (dragging_) {
        int dx = e->pos().x() - dragStart_.x();
        state_.offset = dragOffsetStart_ - dx * state_.scale;
        emit stateChanged();
        update();
    }
}

void Viewport::mouseReleaseEvent(QMouseEvent *e)
{
    if (e->button() == Qt::LeftButton) {
        if (cursorDragging_) {
            cursorDragging_ = false;
        } else if (dragging_) {
            dragging_ = false;
            setCursor(Qt::ArrowCursor);
        }
    }
}

void Viewport::keyPressEvent(QKeyEvent *e)
{
    const double panStep = width() * state_.scale * 0.1;
    switch (e->key()) {
    case Qt::Key_Plus:
    case Qt::Key_Equal:
        zoomAt(width() / 2.0, 0.5);
        break;
    case Qt::Key_Minus:
        zoomAt(width() / 2.0, 2.0);
        break;
    case Qt::Key_Left:
        state_.offset -= panStep;
        emit stateChanged();
        update();
        break;
    case Qt::Key_Right:
        state_.offset += panStep;
        emit stateChanged();
        update();
        break;
    case Qt::Key_Home:
        // Jump to trigger.
        if (!std::isnan(state_.triggerPos)) {
            state_.offset = state_.triggerPos - width() / 2.0 * state_.scale;
            emit stateChanged();
            update();
        }
        break;
    case Qt::Key_C:
        state_.cursorsVisible = !state_.cursorsVisible;
        if (state_.cursorsVisible && state_.cursorA < 0) {
            state_.cursorA = state_.xToTime(width() * 0.3);
            state_.cursorB = state_.xToTime(width() * 0.7);
            emit cursorMoved(state_.cursorA, state_.cursorB);
        }
        emit stateChanged();
        update();
        break;
    default:
        QWidget::keyPressEvent(e);
    }
}

} // namespace openmso::view
