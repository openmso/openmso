#pragma once

#include <QWidget>
#include <QList>
#include <QPointer>

#include "view/ViewState.h"

namespace openmso::view {

class Trace;

// Central waveform area. Owns the trace list, paints them in stacked
// rows, and handles mouse/keyboard interaction (zoom, pan, cursors).
// Per docs/gui-plan/06-rendering.md.
class Viewport : public QWidget {
    Q_OBJECT
public:
    explicit Viewport(QWidget *parent = nullptr);

    void setTraces(const QList<Trace *> &traces);
    void clearTraces();

    ViewState &state() { return state_; }
    const ViewState &state() const { return state_; }

    // Total vertical pixels needed for all traces.
    int contentHeight() const;

signals:
    // Emitted whenever scale/offset/cursors change (so the Ruler and
    // Header can repaint).
    void stateChanged();
    void cursorMoved(double a, double b);

protected:
    void paintEvent(QPaintEvent *e) override;
    void wheelEvent(QWheelEvent *e) override;
    void mousePressEvent(QMouseEvent *e) override;
    void mouseMoveEvent(QMouseEvent *e) override;
    void mouseReleaseEvent(QMouseEvent *e) override;
    void keyPressEvent(QKeyEvent *e) override;

private:
    void zoomAt(double x, double factor);

    QList<QPointer<Trace>> traces_;
    ViewState state_;

    // Drag state.
    bool dragging_ = false;
    QPoint dragStart_;
    double dragOffsetStart_ = 0;
    bool cursorDragging_ = false;
    int cursorDragWhich_ = 0; // 0=A, 1=B
};

} // namespace openmso::view
