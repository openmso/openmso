#pragma once

#include <QObject>
#include <QtGlobal>

namespace openmso::view {

// Single source of truth for the view's horizontal mapping, vertical
// scroll position, and cursors. Owned by TraceView; the Viewport, Ruler
// and Header each hold a non-owning pointer and observe changed() to
// repaint. All mutation goes through the setters below so there is
// exactly one authority (previously Viewport owned a struct and pushed
// copies to the others via setState). Per docs/gui-plan/06-rendering.md.
//
// The horizontal mapping is *clamped to the capture*: you can never zoom
// out past a whole-capture fit or scroll so the data leaves the viewport
// (no blank margins left/right). Clamping needs the data extent
// (setDataSpan) and the viewport width in pixels (setViewportWidth);
// until both are known it is a no-op and the view is free.
class ViewState : public QObject {
    Q_OBJECT
public:
    explicit ViewState(QObject *parent = nullptr) : QObject(parent) {}

    // Horizontal mapping: seconds <-> pixels.
    double scale() const { return scale_; }   // seconds per pixel
    double offset() const { return offset_; }  // seconds at the left edge

    // The capture's time extent (seconds) the view is clamped to.
    double dataStart() const { return dataStart_; }
    double dataEnd() const { return dataEnd_; }
    int viewportWidth() const { return viewportWidth_; }
    // The scale at which the whole capture exactly fills the viewport —
    // i.e. the most zoomed-out the view is allowed to be. 0 if unknown.
    double fitScale() const;

    // Vertical: pixels per trace row (uniform for v0.1).
    int rowHeight() const { return rowHeight_; }

    // Vertical scroll: pixels of trace content scrolled above the top of
    // the viewport. 0 = first trace flush with the top.
    int yOffset() const { return yOffset_; }

    // The row (index into the view's trace list) the user is working
    // with: it is highlighted in the header and viewport, and is the
    // target of cursor edge-snapping and next/prev-edge navigation.
    // -1 = nothing selected.
    int selectedRow() const { return selectedRow_; }

    // Cursors (in seconds). -1 = inactive.
    double cursorA() const { return cursorA_; }
    double cursorB() const { return cursorB_; }
    bool cursorsVisible() const { return cursorsVisible_; }

    // Trigger position in seconds (NaN if unknown).
    double triggerPos() const { return triggerPos_; }

    // Mutators. Each emits changed() only when the resulting view moves
    // (after clamping), so observers never repaint for a no-op and setter
    // loops terminate.
    void setScale(double s);
    void setOffset(double o);
    // Atomic scale+offset update (zoom-about-a-point): keeps the two in a
    // consistent state and emits a single changed().
    void setScaleOffset(double s, double o);
    void setRowHeight(int h);
    void setYOffset(int y);
    void setSelectedRow(int r);
    void setCursors(double a, double b);
    void setCursorsVisible(bool v);
    void setTriggerPos(double t);

    // Clamp bounds. Setting either re-clamps the current scale/offset.
    void setDataSpan(double start, double end);
    void setViewportWidth(int w);

    // Helpers: sample/seconds <-> x pixel.
    double sampleToX(qint64 sample, double samplerate) const {
        return (double(sample) / samplerate - offset_) / scale_;
    }
    qint64 xToSample(double x, double samplerate) const {
        return qint64((x * scale_ + offset_) * samplerate);
    }
    double timeToX(double t) const { return (t - offset_) / scale_; }
    double xToTime(double x) const { return x * scale_ + offset_; }

signals:
    // Any horizontal/vertical/cursor/trigger change. Observers repaint.
    void changed();
    // Cursor positions moved (drives the status-bar readout).
    void cursorMoved(double a, double b);

private:
    static double clampScale(double s);
    // Clamp scale_/offset_ so the view never shows blank space outside
    // [dataStart_, dataEnd_]. No-op until data span + viewport width are
    // both known.
    void clampView();

    double scale_ = 1e-3;
    double offset_ = 0.0;
    double dataStart_ = 0.0;
    double dataEnd_ = 0.0;   // <= dataStart_ ⇒ extent unknown, no clamp.
    int viewportWidth_ = 0;
    int rowHeight_ = 80;
    int yOffset_ = 0;
    int selectedRow_ = -1;
    double cursorA_ = -1.0;
    double cursorB_ = -1.0;
    bool cursorsVisible_ = false;
    double triggerPos_ = qQNaN();
};

} // namespace openmso::view
