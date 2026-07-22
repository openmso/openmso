#include "AnalogSignalTrace.h"

#include "data/Signal.h"

#include <QPainter>
#include <QPainterPath>
#include <algorithm>
#include <cmath>
#include <limits>

namespace openmso::view {

AnalogSignalTrace::AnalogSignalTrace(data::Signal *sig, QObject *parent)
    : SignalTrace(sig, parent) {}

void AnalogSignalTrace::paintMid(QPainter &p, const QRect &rect,
                                 const ViewState &st)
{
    if (!sig_) return;
    auto *seg = qobject_cast<data::AnalogSegment *>(sig_->primarySegment());
    if (!seg) return;

    const double sr = seg->samplerate();
    if (sr <= 0) return;
    const qint64 total = seg->appendedSamples();
    if (total == 0) return;

    const qint64 first = std::max(qint64(0), st.xToSample(rect.left(), sr));
    const qint64 last  = std::min(total - 1, st.xToSample(rect.right(), sr));
    if (last < first) return;

    const double samplesPerPixel = sr * st.scale();

    // Determine value range across the WHOLE capture (not just the
    // visible window) so the amplitude is stable while panning/zooming X.
    double vmin, vmax;
    fullRange(*seg, vmin, vmax);
    if (vmax - vmin < 1e-9) { vmax = vmin + 1e-9; }
    const double padding = (vmax - vmin) * 0.1;
    vmin -= padding; vmax += padding;

    auto yFor = [&](double v) {
        double t = (v - vmin) / (vmax - vmin);  // 0..1, 1=high
        return double(rect.bottom()) - t * rect.height();
    };

    p.setPen(QPen(color_, 1));

    if (samplesPerPixel < 1.0) {
        // Per-sample polyline.
        QPainterPath path;
        for (qint64 s = first; s <= last; ++s) {
            double x = st.sampleToX(s, sr);
            double y = yFor(seg->sampleAt(s));
            if (s == first) path.moveTo(x, y);
            else path.lineTo(x, y);
        }
        p.drawPath(path);
    } else {
        // Envelope: one vertical min/max bar per pixel column.
        const auto &env = seg->envelope();
        int level = env.levelForSamplePerPixel(samplesPerPixel);
        if (level < 0) {
            // Fall back to per-sample.
            QPainterPath path;
            for (qint64 s = first; s <= last; ++s) {
                double x = st.sampleToX(s, sr);
                double y = yFor(seg->sampleAt(s));
                if (s == first) path.moveTo(x, y);
                else path.lineTo(x, y);
            }
            p.drawPath(path);
        } else {
            const auto &L = env.level(level);
            qint64 bucket = L.bucketSize;
            qint64 firstBucket = first / bucket;
            qint64 lastBucket = last / bucket;
            for (qint64 b = firstBucket; b <= lastBucket; ++b) {
                if (b < 0 || b >= L.minima.size()) continue;
                double x = st.sampleToX(b * bucket, sr);
                double yMin = yFor(L.minima[b] * seg->scale() + seg->offset());
                double yMax = yFor(L.maxima[b] * seg->scale() + seg->offset());
                p.drawLine(QPointF(x, yMin), QPointF(x, yMax));
            }
        }
    }
}

void AnalogSignalTrace::fullRange(const data::AnalogSegment &seg,
                                  double &vmin, double &vmax) const
{
    const qint64 total = seg.appendedSamples();
    if (total <= 0) { vmin = 0.0; vmax = 1.0; return; }
    // Cache: only recompute when more samples have arrived.
    if (total != cachedForSamples_) {
        valueRange(seg, 0, total - 1, cachedVmin_, cachedVmax_);
        cachedForSamples_ = total;
    }
    vmin = cachedVmin_;
    vmax = cachedVmax_;
}

void AnalogSignalTrace::valueRange(const data::AnalogSegment &seg,
                                   qint64 first, qint64 last,
                                   double &vmin, double &vmax) const
{
    vmin = std::numeric_limits<double>::infinity();
    vmax = -std::numeric_limits<double>::infinity();
    // For modest visible ranges, scan samples directly. For very wide
    // ranges we'd use the envelope; defer to a later milestone.
    const qint64 step = std::max<qint64>(1, (last - first) / 4000);
    for (qint64 s = first; s <= last; s += step) {
        double v = seg.sampleAt(s);
        if (v < vmin) vmin = v;
        if (v > vmax) vmax = v;
    }
}

} // namespace openmso::view
