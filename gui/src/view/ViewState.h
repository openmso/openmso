#pragma once

#include <QColor>
#include <QPointF>
#include <QtGlobal>

namespace openmso::view {

// Shared viewport state. Any of {Viewport, Ruler, Header} may mutate
// scale/offset/cursors; the others observe via signals in TraceView.
// Per docs/gui-plan/06-rendering.md.
struct ViewState {
    // Horizontal mapping: seconds <-> pixels.
    double scale = 1e-3;        // seconds per pixel
    double offset = 0.0;        // seconds at the left edge

    // Vertical: pixels per trace row (uniform for v0.1).
    int rowHeight = 80;

    // Vertical scroll position: pixels of trace content scrolled above
    // the top of the viewport. 0 = first trace flush with the top.
    int yOffset = 0;

    // Cursors (in seconds). -1 = inactive.
    double cursorA = -1.0;
    double cursorB = -1.0;
    bool cursorsVisible = false;

    // Trigger position in seconds (NaN if unknown).
    double triggerPos = qQNaN();

    // Helper: sample index -> x pixel.
    double sampleToX(qint64 sample, double samplerate) const {
        return (double(sample) / samplerate - offset) / scale;
    }
    // Helper: x pixel -> sample index.
    qint64 xToSample(double x, double samplerate) const {
        return qint64((x * scale + offset) * samplerate);
    }
    // Helper: seconds -> x pixel.
    double timeToX(double t) const { return (t - offset) / scale; }
    double xToTime(double x) const { return x * scale + offset; }
};

} // namespace openmso::view
