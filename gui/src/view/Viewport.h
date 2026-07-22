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

    // Inject the shared view state (owned by TraceView). The viewport
    // reads it every paint and mutates it through its setters; it also
    // repaints itself whenever the state signals changed().
    void setViewState(ViewState *st);

    void setTraces(const QList<Trace *> &traces);
    void clearTraces();

    // Zoom the time axis around the viewport center (factor < 1 zooms
    // in). Toggle the cursor pair on/off.
    void zoom(double factor);
    void toggleCursors();

    // Total vertical pixels needed for all traces.
    int contentHeight() const;

protected:
    void paintEvent(QPaintEvent *e) override;
    void wheelEvent(QWheelEvent *e) override;
    void mousePressEvent(QMouseEvent *e) override;
    void mouseMoveEvent(QMouseEvent *e) override;
    void mouseReleaseEvent(QMouseEvent *e) override;
    void keyPressEvent(QKeyEvent *e) override;

private:
    void zoomAt(double x, double factor);
    // Horizontal pan by a pixel delta (positive = content moves left).
    void panPixels(double dxPixels);

    QList<QPointer<Trace>> traces_;
    ViewState *state_ = nullptr;   // shared, owned by TraceView.

    // Cursor selection: left-drag lays down a time range (like selecting
    // audio in Audacity), anchored where the press landed.
    bool selecting_ = false;
    double selAnchor_ = 0.0;   // seconds.
};

} // namespace openmso::view
