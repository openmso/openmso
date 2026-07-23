#pragma once

#include <QFrame>
#include <QList>
#include <QPointer>

#include "view/ViewState.h"

class QSplitter;
class QScrollBar;

namespace openmso::data { class Capture; class Signal; }

namespace openmso::view {

class Header;
class Ruler;
class Viewport;
class Trace;

// Composite widget: Header (left) + Ruler/Viewport (right). Owns the
// trace list and the shared ViewState. Per docs/gui-plan/06-rendering.md.
class TraceView : public QFrame {
    Q_OBJECT
public:
    explicit TraceView(QWidget *parent = nullptr);

    // Rebuild the trace list from a capture. The view watches the
    // capture for dataChanged and repaints.
    void setCapture(data::Capture *cap);

    ViewState *state() { return state_; }

    // The data::Signal backing the currently selected row, or nullptr if
    // nothing is selected. Used by the measurement dock.
    data::Signal *selectedSignal() const;

    // Set the time scale so the whole capture fits the viewport width,
    // with the start of the data at the left edge.
    void fitToData();

    // View commands (delegate to the Viewport, which owns the state).
    void zoomIn();
    void zoomOut();
    void toggleCursors();
    // Move cursor A to the next/previous edge on the selected channel.
    void nextEdge();
    void prevEdge();

signals:
    void cursorMoved(double a, double b);
    // Capture data changed (chunk appended or capture ended) — the
    // measurement dock recomputes on this and on ViewState::changed.
    void dataChanged();

protected:
    void resizeEvent(QResizeEvent *e) override;

private:
    void rebuildTraces();
    void onDataChanged();
    void syncScrollBars();
    // Push the capture's current time extent into the ViewState so the
    // horizontal zoom/scroll stays clamped to the data.
    void updateDataSpan();

    ViewState *state_;   // single source of truth, owned here.
    Header *header_;
    Ruler *ruler_;
    Viewport *viewport_;
    QScrollBar *vscroll_;
    QScrollBar *hscroll_;
    QPointer<data::Capture> capture_;
    QList<QPointer<Trace>> traces_;
};

} // namespace openmso::view
