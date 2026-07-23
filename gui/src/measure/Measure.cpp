#include "Measure.h"

#include "data/AnalogSegment.h"
#include "data/LogicSegment.h"

#include <QReadLocker>

#include <algorithm>
#include <cmath>
#include <limits>

namespace openmso::measure {

using namespace openmso::data;

AnalogStats measureAnalog(const AnalogSegment &seg, qint64 first, qint64 last)
{
    AnalogStats st;
    st.unit = seg.unit();

    // Take the read lock once and decode inline, rather than paying a
    // lock per sample through sampleAt() — measurement windows can be
    // millions of samples wide.
    QReadLocker l(&seg.lock);
    const qint64 n = seg.appendedSamples();
    first = std::max<qint64>(0, first);
    last = std::min<qint64>(n, last);
    if (last <= first) return st;

    const int bps = seg.bytesPerSample();
    const char *base = seg.rawBytes().constData();
    const AnalogDType dt = seg.dtype();
    const double scale = seg.scale();
    const double offset = seg.offset();

    double mn = std::numeric_limits<double>::infinity();
    double mx = -std::numeric_limits<double>::infinity();
    double sum = 0.0;
    double sumSq = 0.0;
    for (qint64 s = first; s < last; ++s) {
        const double v = decodeSample(dt, base + s * bps, scale, offset);
        mn = std::min(mn, v);
        mx = std::max(mx, v);
        sum += v;
        sumSq += v * v;
    }

    const qint64 cnt = last - first;
    st.valid = true;
    st.sampleCount = cnt;
    st.min = mn;
    st.max = mx;
    st.pp = mx - mn;
    st.mean = sum / double(cnt);
    st.rms = std::sqrt(sumSq / double(cnt));
    return st;
}

LogicStats measureLogic(const LogicSegment &seg, int bit,
                        qint64 first, qint64 last, double samplerate)
{
    LogicStats st;
    const qint64 n = seg.appendedSamples();
    first = std::max<qint64>(0, first);
    last = std::min<qint64>(n, last);
    if (last <= first || samplerate <= 0.0) return st;

    // edgeIndex() manages its own locking (and lazily builds); do not hold
    // an outer lock here or we'd deadlock against its write lock.
    bool level = false;   // value just before `first`.
    const auto edges = seg.edgeIndex().edgesInRange(bit, first, last, &level);
    st.valid = true;
    st.edgeCount = edges.size();

    // Walk the edges, tracking level, to collect rising edges (for period)
    // and high-pulse widths (for duty and pulse-width min/max).
    qint64 firstRise = -1, lastRise = -1, riseCount = 0, pendingRise = -1;
    double highSampleSum = 0.0;
    qint64 highPulses = 0;
    qint64 posMin = std::numeric_limits<qint64>::max();
    qint64 posMax = 0;
    for (const qint64 e : edges) {
        const bool newLevel = !level;
        if (newLevel) {                     // rising edge: 0 → 1
            if (firstRise < 0) firstRise = e;
            lastRise = e;
            ++riseCount;
            pendingRise = e;
        } else if (pendingRise >= 0) {       // falling edge closing a pulse
            const qint64 w = e - pendingRise;
            highSampleSum += double(w);
            ++highPulses;
            posMin = std::min(posMin, w);
            posMax = std::max(posMax, w);
            pendingRise = -1;
        }
        level = newLevel;
    }

    if (riseCount >= 2) {
        const double periodSamples =
            double(lastRise - firstRise) / double(riseCount - 1);
        if (periodSamples > 0.0) {
            st.hasTiming = true;
            st.period = periodSamples / samplerate;
            st.frequency = samplerate / periodSamples;
            if (highPulses > 0) {
                const double meanHigh = highSampleSum / double(highPulses);
                st.dutyCycle = std::clamp(meanHigh / periodSamples, 0.0, 1.0);
            }
        }
    }
    if (highPulses > 0) {
        st.posWidthMin = double(posMin) / samplerate;
        st.posWidthMax = double(posMax) / samplerate;
    }
    return st;
}

} // namespace openmso::measure
