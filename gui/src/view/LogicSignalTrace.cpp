#include "LogicSignalTrace.h"

#include "data/Signal.h"

#include <QPainter>
#include <QPainterPath>

namespace openmso::view {

LogicSignalTrace::LogicSignalTrace(data::Signal *sig, int bitIndex,
                                   QObject *parent)
    : SignalTrace(sig, parent), bitIndex_(bitIndex) {}

void LogicSignalTrace::paintMid(QPainter &p, const QRect &rect,
                                const ViewState &st)
{
    if (!sig_) return;
    auto *seg = qobject_cast<data::LogicSegment *>(sig_->primarySegment());
    if (!seg) return;

    const double sr = seg->samplerate();
    if (sr <= 0) return;
    const qint64 total = seg->appendedSamples();
    if (total == 0) return;

    const int high_y = rect.top() + rect.height() * 2 / 8;
    const int low_y  = rect.top() + rect.height() * 6 / 8;

    // Visible sample range, extended one sample past each edge so the
    // trace reaches the screen borders instead of stopping at the last
    // fully-visible sample.
    const qint64 first = std::max(qint64(0), st.xToSample(rect.left(), sr) - 1);
    const qint64 last  = std::min(total - 1, st.xToSample(rect.right(), sr) + 1);
    if (last < first) return;

    const double samplesPerPixel = sr * st.scale();

    p.setPen(QPen(color_, 1));

    // Lazy edge index handles the zoomed-out case.
    const auto &idx = seg->edgeIndex();

    bool prevValue = false;
    auto edges = idx.edgesInRange(bitIndex_, first, last + 1, &prevValue);

    // Walk the visible range edge by edge.
    QPainterPath path;
    double x = st.sampleToX(first, sr);
    int y = prevValue ? high_y : low_y;
    path.moveTo(x, y);
    for (qint64 e : edges) {
        double xe = st.sampleToX(e, sr);
        // Vertical transition at e.
        path.lineTo(xe, y);
        prevValue = !prevValue;
        y = prevValue ? high_y : low_y;
        path.lineTo(xe, y);
    }
    // The level holds until the next edge, which is off-screen right at
    // this point — continue the line to the right screen border rather
    // than stopping at the last sample (which left a blank strip).
    path.lineTo(double(rect.right()) + 1.0, y);
    p.drawPath(path);

    // When zoomed in past ~2 samples/pixel, also draw the individual
    // sample points so the user sees the discrete sampling.
    if (samplesPerPixel < 0.5) {
        // Per-sample polyline already covers this; the path above is
        // sufficient. (A real "dots" mode could be added here.)
    }
}

} // namespace openmso::view
