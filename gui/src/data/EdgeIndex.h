#pragma once

#include <QVector>
#include <QtGlobal>

namespace openmso::data {

// Lazy-built index of logic transitions. For each channel (bit) within
// a packed logic segment, holds the list of sample indices where the
// bit changed value. Derived from first principles (not copied from
// PulseView): walk the packed bytes, XOR each sample with its
// predecessor, and record any bit that flipped.
//
// The painter asks for edges in a [first, last) sample range; we
// binary-search the per-bit sorted index. Memory is O(edges), which
// is bounded by signal activity, not sample count.
class EdgeIndex {
public:
    // Build the index for all `nchans` bits packed in `data` (unitsize
    // bytes per sample, `nsamples` samples). Called lazily on first
    // paint after data changes.
    void build(const QByteArray &data, int unitsize, int nchans,
               qint64 nsamples);

    int channelCount() const { return edges_.size(); }

    // Return all edge sample indices for `chan` in [first, last).
    // If `prevValue` is non-null, set to the bit value just before
    // `first` (so the painter knows whether to start high or low).
    QVector<qint64> edgesInRange(int chan, qint64 first, qint64 last,
                                 bool *prevValue = nullptr) const;

    // Single-edge queries for cursor snapping and edge navigation. All
    // return a sample index, or -1 if there is no such edge. `nextEdge`
    // and `prevEdge` are *strict* (an edge sitting exactly on `after` /
    // `before` is skipped, so repeated calls step through transitions);
    // `nearestEdge` is inclusive (an edge on `sample` returns `sample`).
    qint64 nextEdge(int chan, qint64 after) const;
    qint64 prevEdge(int chan, qint64 before) const;
    qint64 nearestEdge(int chan, qint64 sample) const;

private:
    // edges_[chan] = sorted sample indices where bit `chan` flipped.
    QVector<QVector<qint64>> edges_;
    // Value of each bit at sample 0 (so we can compute "value just
    // before first" by counting edges before `first`).
    QVector<bool> initialValues_;
};

} // namespace openmso::data
