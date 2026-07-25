#include "LogicSignalTrace.h"

#include "PaintProfile.h"
#include "data/Signal.h"

#include <QDebug>
#include <QElapsedTimer>
#include <QPainter>

#include <cmath>

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

    // Cosmetic (width-0) pen — critical for HiDPI. Under a fractional (or
    // any non-integer) devicePixelRatio the painter carries a scale
    // transform; a NON-cosmetic width-1 pen must then be stroked into a
    // device-space polygon and filled, which goes quadratic on a dense
    // trace of overlapping vertical edges (measured: ~235ms per lane at
    // dpr=1.3333, vs ~150us cosmetic — see docs profiling notes). A
    // cosmetic pen is 1 device pixel regardless of transform and takes the
    // fast direct-line path, so it stays cheap at any scale factor.
    QPen pen(color_);
    pen.setCosmetic(true);
    p.setPen(pen);

    // Lazy edge index handles the zoomed-out case.
    const auto &idx = seg->edgeIndex();

    const bool prof = paintProfileEnabled();
    QElapsedTimer timer;
    if (prof) timer.start();

    bool prevValue = false;
    auto edges = idx.edgesInRange(bitIndex_, first, last + 1, &prevValue);

    const qint64 fetch_us = prof ? timer.nsecsElapsed() / 1000 : 0;
    if (prof) timer.restart();

    // Walk the visible range, decimating to at most one vertical stroke per
    // horizontal pixel column. Zoomed out, thousands of edges can land in a
    // single column; drawing them all buries the raster engine in sub-pixel
    // segments (see profiling) while producing no visible detail. Instead we
    // coalesce every edge sharing an integer x column into one stroke.
    //
    // Correctness note: the trace stays a *single continuous* polyline (every
    // point is connected, no gaps), so adjacent columns are always joined by
    // a vertical connector — this is what stops the "low block, then high
    // block, no rising edge between them" discontinuity. When a column toggles
    // an even number of times it exits at the level it entered; emitting
    // entry->exit alone would erase that excursion, so we force a full-height
    // spike whenever entry == exit but the column had any edge.
    //
    // Rendering note: logic traces are strictly axis-aligned, so we draw the
    // polyline with antialiasing OFF (see profiling — AA'd rasterization here
    // meant gamma-correct per-pixel alpha blending with heavy overdraw, ~83ms
    // for a 2.5k-point path; aliased axis-aligned 1px lines look identical and
    // cost ~nothing). drawPolyline also skips the QPainterPath stroker.
    QVector<QPointF> pts;
    pts.reserve(int(edges.size()) + 4);
    bool val = prevValue;
    double lastX = st.sampleToX(first, sr);
    int y = val ? high_y : low_y;
    pts.append(QPointF(lastX, y));

    const qsizetype n = edges.size();
    qsizetype i = 0;
    while (i < n) {
        const int col = int(std::floor(st.sampleToX(edges[i], sr)));
        const bool entryVal = val;
        // Consume every edge in this column, tracking the net level.
        do {
            val = !val;
            ++i;
        } while (i < n && int(std::floor(st.sampleToX(edges[i], sr))) == col);

        // Never step left (guards a sub-pixel backtrack for the first
        // column, whose floor() can land just left of the start sample).
        const double colX = std::max(lastX, double(col));
        const int entryY = entryVal ? high_y : low_y;

        pts.append(QPointF(colX, entryY));            // flat run up to column
        if (val != entryVal) {
            pts.append(QPointF(colX, val ? high_y : low_y));   // net transition
        } else {
            // Toggled but returned: show the excursion so the edge is visible.
            pts.append(QPointF(colX, entryVal ? low_y : high_y));
            pts.append(QPointF(colX, entryY));
        }
        lastX = colX;
        y = val ? high_y : low_y;
    }
    // The level holds until the next edge, which is off-screen right at
    // this point — continue the line to the right screen border rather
    // than stopping at the last sample (which left a blank strip).
    pts.append(QPointF(double(rect.right()) + 1.0, y));

    const qint64 build_us = prof ? timer.nsecsElapsed() / 1000 : 0;
    if (prof) timer.restart();

    const bool wasAA = p.testRenderHint(QPainter::Antialiasing);
    p.setRenderHint(QPainter::Antialiasing, false);
    p.drawPolyline(pts.constData(), int(pts.size()));
    p.setRenderHint(QPainter::Antialiasing, wasAA);

    if (prof) {
        const qint64 draw_us = timer.nsecsElapsed() / 1000;
        qDebug().nospace()
            << "[paint-prof] logic \"" << sig_->name() << "\" bit " << bitIndex_
            << " build=" << build_us << "us"
            << " edges=" << edges.size()
            << " segs=" << pts.size()
            << " span=" << (last - first + 1) << "smp"
            << " spp=" << samplesPerPixel
            << " fetch=" << fetch_us << "us"
            << " draw=" << draw_us << "us"
            << " rect=" << rect.width() << "x" << rect.height()
            << " dpr=" << (p.device() ? p.device()->devicePixelRatioF() : 1.0)
            << " clip=" << p.hasClipping();
    }

    // When zoomed in past ~2 samples/pixel, also draw the individual
    // sample points so the user sees the discrete sampling.
    if (samplesPerPixel < 0.5) {
        // Per-sample polyline already covers this; the path above is
        // sufficient. (A real "dots" mode could be added here.)
    }
}

} // namespace openmso::view
