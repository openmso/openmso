#include "AnalogSignalTrace.h"

#include "PaintProfile.h"
#include "data/Signal.h"

#include <QDebug>
#include <QElapsedTimer>
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

    // Extend one sample past each edge so the polyline reaches the screen
    // borders — a partial line to the off-screen neighbour — instead of
    // stopping at the last fully-visible sample (which left a blank strip
    // on the right at deep zoom).
    const qint64 first = std::max(qint64(0), st.xToSample(rect.left(), sr) - 1);
    const qint64 last  = std::min(total - 1, st.xToSample(rect.right(), sr) + 1);
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

    // Cosmetic (width-0) pen — see LogicSignalTrace for the full rationale:
    // under a non-integer devicePixelRatio the painter carries a scale
    // transform, and a non-cosmetic width-1 pen is then stroked into a
    // device-space polygon whose fill goes quadratic on dense traces. A
    // cosmetic pen stays 1 device pixel at any scale and is cheap.
    QPen pen(color_);
    pen.setCosmetic(true);
    p.setPen(pen);

    const bool prof = paintProfileEnabled();
    QElapsedTimer timer;
    if (prof) timer.start();

    const char *mode = "persample";
    qint64 segs = 0;

    auto drawPerSample = [&] {
        // One connected polyline, one drawPolyline call (batched).
        QVector<QPointF> pts;
        pts.reserve(int(last - first) + 1);
        for (qint64 s = first; s <= last; ++s)
            pts.append(QPointF(st.sampleToX(s, sr), yFor(seg->sampleAt(s))));
        segs = pts.size();
        p.drawPolyline(pts.constData(), int(pts.size()));
    };

    if (samplesPerPixel < 1.0) {
        drawPerSample();
    } else {
        // Envelope: one vertical min/max bar per pixel column, plus min/max
        // connectors to the previous column so flat runs stay a continuous
        // line. All segments are accumulated and issued in a single
        // drawLines call rather than ~3 drawLine calls per column.
        const auto &env = seg->envelope();
        int level = env.levelForSamplePerPixel(samplesPerPixel);
        if (level < 0) {
            drawPerSample();
        } else {
            mode = "envelope";
            const auto &L = env.level(level);
            qint64 bucket = L.bucketSize;
            qint64 firstBucket = first / bucket;
            qint64 lastBucket = last / bucket + 1;   // one past the right edge.
            QVector<QLineF> lines;
            lines.reserve(int(lastBucket - firstBucket + 1) * 3);
            bool have = false;
            double px = 0, pMinY = 0, pMaxY = 0;
            for (qint64 b = firstBucket; b <= lastBucket; ++b) {
                if (b < 0 || b >= L.minima.size()) continue;
                double x = st.sampleToX(b * bucket, sr);
                double yMin = yFor(L.minima[b] * seg->scale() + seg->offset());
                double yMax = yFor(L.maxima[b] * seg->scale() + seg->offset());
                lines.append(QLineF(x, yMin, x, yMax));
                if (have) {
                    lines.append(QLineF(px, pMaxY, x, yMax));
                    lines.append(QLineF(px, pMinY, x, yMin));
                }
                px = x; pMinY = yMin; pMaxY = yMax; have = true;
            }
            segs = lines.size();
            p.drawLines(lines.constData(), int(lines.size()));
        }
    }

    if (prof) {
        qDebug().nospace()
            << "[paint-prof] analog \"" << sig_->name() << "\""
            << " mode=" << mode
            << " segs=" << segs
            << " span=" << (last - first + 1) << "smp"
            << " spp=" << samplesPerPixel
            << " draw=" << (timer.nsecsElapsed() / 1000) << "us"
            << " rect=" << rect.width() << "x" << rect.height()
            << " dpr=" << (p.device() ? p.device()->devicePixelRatioF() : 1.0);
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
