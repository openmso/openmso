#pragma once

#include <QFrame>
#include <QList>
#include <QPointer>

#include "view/ViewState.h"

class QSplitter;
class QScrollBar;

namespace openmso::data { class Capture; }

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

    ViewState &state() { return state_; }

    // Set the time scale so the whole capture fits the viewport width,
    // with the start of the data at the left edge.
    void fitToData();

    // View commands (delegate to the Viewport, which owns the state).
    void zoomIn();
    void zoomOut();
    void toggleCursors();

signals:
    void cursorMoved(double a, double b);

protected:
    void resizeEvent(QResizeEvent *e) override;

private:
    void rebuildTraces();
    void onStateChanged();
    void onDataChanged();
    void syncScrollBar();

    Header *header_;
    Ruler *ruler_;
    Viewport *viewport_;
    QScrollBar *vscroll_;
    QPointer<data::Capture> capture_;
    QList<QPointer<Trace>> traces_;
    ViewState state_;
};

} // namespace openmso::view
