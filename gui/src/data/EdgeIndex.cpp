#include "EdgeIndex.h"

#include <algorithm>

namespace openmso::data {

namespace {

// Read bit `chan` out of a packed sample at `data + sample*unitsize`.
inline bool bitAt(const char *data, int unitsize, int chan, qint64 sample)
{
    const quint8 byte =
        *reinterpret_cast<const quint8 *>(data + sample * unitsize
                                          + (chan / 8));
    return (byte >> (chan % 8)) & 1u;
}

} // namespace

void EdgeIndex::build(const QByteArray &data, int unitsize, int nchans,
                      qint64 nsamples)
{
    edges_.clear();
    initialValues_.clear();
    edges_.resize(nchans);
    initialValues_.resize(nchans);

    if (nsamples == 0 || data.isEmpty())
        return;

    const char *base = data.constData();
    // Record initial values.
    for (int c = 0; c < nchans; ++c)
        initialValues_[c] = bitAt(base, unitsize, c, 0);

    // For each sample from 1..nsamples-1, XOR against predecessor.
    // Per-channel: if bit differs, record edge at this sample.
    // This is O(nsamples * nchans / 8) — a byte-wise XOR then bit scan
    // would be faster, but this is clear and fast enough for the demo.
    for (qint64 s = 1; s < nsamples; ++s) {
        const char *prev = base + (s - 1) * unitsize;
        const char *curr = base + s * unitsize;
        // Compare bytes that contain any of our channels.
        int nbytes = (nchans + 7) / 8;
        for (int b = 0; b < nbytes; ++b) {
            quint8 diff = quint8(curr[b]) ^ quint8(prev[b]);
            if (!diff) continue;
            // Scan set bits.
            for (int bit = 0; bit < 8; ++bit) {
                if (diff & (1u << bit)) {
                    int chan = b * 8 + bit;
                    if (chan < nchans)
                        edges_[chan].append(s);
                }
            }
        }
    }
}

QVector<qint64> EdgeIndex::edgesInRange(int chan, qint64 first, qint64 last,
                                        bool *prevValue) const
{
    QVector<qint64> out;
    if (chan < 0 || chan >= edges_.size())
        return out;
    const auto &v = edges_[chan];

    // Value just before `first` = initial XOR parity of edges before it.
    if (prevValue) {
        bool val = initialValues_.value(chan);
        auto it = std::lower_bound(v.begin(), v.end(), first);
        int count = int(it - v.begin());
        if (count % 2) val = !val;
        *prevValue = val;
    }
    auto lo = std::lower_bound(v.begin(), v.end(), first);
    auto hi = std::lower_bound(v.begin(), v.end(), last);
    out.reserve(int(hi - lo));
    for (auto it = lo; it != hi; ++it)
        out.append(*it);
    return out;
}

qint64 EdgeIndex::nextEdge(int chan, qint64 after) const
{
    if (chan < 0 || chan >= edges_.size()) return -1;
    const auto &v = edges_[chan];
    auto it = std::upper_bound(v.begin(), v.end(), after);
    return it == v.end() ? -1 : *it;
}

qint64 EdgeIndex::prevEdge(int chan, qint64 before) const
{
    if (chan < 0 || chan >= edges_.size()) return -1;
    const auto &v = edges_[chan];
    auto it = std::lower_bound(v.begin(), v.end(), before);
    return it == v.begin() ? -1 : *(it - 1);
}

qint64 EdgeIndex::nearestEdge(int chan, qint64 sample) const
{
    if (chan < 0 || chan >= edges_.size()) return -1;
    const auto &v = edges_[chan];
    if (v.isEmpty()) return -1;
    auto it = std::lower_bound(v.begin(), v.end(), sample);
    if (it == v.end()) return v.last();          // past the last edge.
    if (*it == sample) return sample;            // exact hit.
    if (it == v.begin()) return *it;             // before the first edge.
    const qint64 hi = *it, lo = *(it - 1);       // straddling neighbours.
    return (sample - lo <= hi - sample) ? lo : hi;
}

} // namespace openmso::data
