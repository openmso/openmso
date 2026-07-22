#pragma once

#include "data/AnalogSegment.h"
#include "view/SignalTrace.h"

namespace openmso::view {

// Renders an analog channel. Per 06-rendering.md.
class AnalogSignalTrace : public SignalTrace {
    Q_OBJECT
public:
    AnalogSignalTrace(data::Signal *sig, QObject *parent = nullptr);

    void paintMid(QPainter &p, const QRect &rect,
                  const ViewState &st) override;

private:
    // Vertical mapping. The view layer auto-fits the trace's value
    // range to the row height.
    void valueRange(const data::AnalogSegment &seg, qint64 first,
                    qint64 last, double &vmin, double &vmax) const;

    // Value range over the *whole* capture, cached so panning/zooming X
    // doesn't rescale the amplitude (which turned e.g. a flat square-wave
    // top into "noise"). Recomputed only when the sample count grows.
    void fullRange(const data::AnalogSegment &seg, double &vmin,
                   double &vmax) const;
    mutable double cachedVmin_ = 0.0;
    mutable double cachedVmax_ = 0.0;
    mutable qint64 cachedForSamples_ = -1;
};

} // namespace openmso::view
