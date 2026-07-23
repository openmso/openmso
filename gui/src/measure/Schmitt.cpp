#include "Schmitt.h"

namespace openmso::measure {

using namespace openmso::data;

namespace {

// Suppress runs shorter than `minLen` samples by absorbing them into the
// preceding run's level, then re-checking the merged span (so a train of
// short runs collapses correctly). O(n) in the common case; a pathological
// alternating input degrades but this runs off the GUI thread. The very
// first run has no predecessor, so it is left as-is.
void deglitch(QByteArray &b, qint64 minLen)
{
    if (minLen <= 1 || b.size() < 2) return;
    const qint64 n = b.size();
    char *o = b.data();
    qint64 i = 0;
    while (i < n) {
        qint64 j = i;
        while (j < n && o[j] == o[i]) ++j;   // run is [i, j)
        if (j - i < minLen && i > 0) {
            const char prev = o[i - 1];
            for (qint64 k = i; k < j; ++k) o[k] = prev;
            // Re-scan from the start of the (now extended) predecessor run
            // so it can merge with a following same-level run and have its
            // combined length re-checked.
            qint64 p = i - 1;
            while (p > 0 && o[p - 1] == prev) --p;
            i = p;
        } else {
            i = j;
        }
    }
}

} // namespace

QByteArray schmittWalk(const QByteArray &raw, AnalogDType dtype,
                       double scale, double offset, qint64 nsamples,
                       const SchmittParams &p)
{
    const int bps = bytesPerSample(dtype);
    qint64 n = nsamples;
    if (bps > 0)
        n = std::min<qint64>(n, raw.size() / bps);
    if (n <= 0 || bps <= 0) return {};

    QByteArray out(n, Qt::Uninitialized);
    char *o = out.data();
    const char *base = raw.constData();
    const double mid = 0.5 * (p.vHigh + p.vLow);

    // Seed the level from the first sample vs the midpoint, so a trace that
    // starts already above/below its thresholds settles immediately instead
    // of waiting for the first crossing.
    bool level = decodeSample(dtype, base, scale, offset) > mid;
    for (qint64 s = 0; s < n; ++s) {
        const double v = decodeSample(dtype, base + s * bps, scale, offset);
        if (!level && v >= p.vHigh) level = true;
        else if (level && v <= p.vLow) level = false;
        o[s] = char((level != p.invert) ? 1 : 0);
    }

    if (p.deglitchSamples > 1)
        deglitch(out, p.deglitchSamples);
    return out;
}

} // namespace openmso::measure
