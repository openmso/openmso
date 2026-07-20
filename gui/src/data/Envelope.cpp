#include "Envelope.h"

#include "Types.h"

#include <algorithm>
#include <cmath>

namespace openmso::data {

namespace {

// Decode all samples into a vector of doubles. For 8-bit captures
// this is 8 bytes/sample of working memory — acceptable for the
// demo (100k samples = 800 KB). For 14M-sample hardware captures we'd
// decode lazily per bucket instead; deferred to v0.2.
QVector<double> decodeAll(const QByteArray &data, int bps, AnalogDType dt)
{
    const int n = data.size() / bps;
    QVector<double> out;
    out.resize(n);
    const char *p = data.constData();
    for (int i = 0; i < n; ++i)
        out[i] = decodeSample(dt, p + i * bps, 1.0, 0.0); // raw codes
    return out;
}

} // namespace

void Envelope::build(const QByteArray &data, int bps, AnalogDType dt,
                     qint64 nsamples)
{
    levels_.clear();
    if (nsamples == 0 || data.isEmpty())
        return;

    // Level 0: bucket = 1 sample (min=max=sample). We skip building
    // level 0 explicitly — the painter draws per-sample when zoomed
    // past the deepest level. Level 1 has bucket=2, level 2 bucket=4,
    // etc. We stop when a level has <= 1 bucket.
    QVector<double> samples = decodeAll(data, bps, dt);
    const double *s = samples.constData();
    qint64 n = samples.size();

    int level = 1;
    while (true) {
        qint64 bucket = qint64(1) << level;
        if (bucket > n) break;
        qint64 nbuckets = (n + bucket - 1) / bucket;
        Level L;
        L.bucketSize = bucket;
        L.minima.resize(nbuckets);
        L.maxima.resize(nbuckets);
        for (qint64 i = 0; i < nbuckets; ++i) {
            double mn = std::numeric_limits<double>::infinity();
            double mx = -mn;
            qint64 lo = i * bucket;
            qint64 hi = std::min(lo + bucket, n);
            for (qint64 j = lo; j < hi; ++j) {
                if (s[j] < mn) mn = s[j];
                if (s[j] > mx) mx = s[j];
            }
            L.minima[i] = mn;
            L.maxima[i] = mx;
        }
        levels_.append(std::move(L));
        ++level;
        if (level > 24) break; // sanity cap
    }
}

int Envelope::levelForSamplePerPixel(double samplesPerPixel) const
{
    // Pick the deepest level whose bucketSize <= samplesPerPixel.
    // If samplesPerPixel < 2 (level 1's bucket), no envelope helps;
    // the painter should draw per-sample.
    for (int i = levels_.size() - 1; i >= 0; --i) {
        if (double(levels_[i].bucketSize) <= samplesPerPixel)
            return i;
    }
    return -1;
}

} // namespace openmso::data
