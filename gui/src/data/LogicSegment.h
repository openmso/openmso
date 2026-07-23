#pragma once

#include <QByteArray>
#include <QReadWriteLock>

#include "EdgeIndex.h"
#include "Segment.h"

namespace openmso::data {

// Bit-packed logic samples, byte-compatible with OCP capture.data
// (SR_DF_LOGIC layout): `unitsize` bytes per sample, channel i = bit i
// across the unit. Per 05-data-model.md.
//
// Chunks are appended verbatim as they arrive from the wire. The edge
// index is built lazily on first query after data changes.
class LogicSegment : public Segment {
    Q_OBJECT
public:
    LogicSegment(int unitsize, int nchans, QObject *parent = nullptr);

    int unitsize() const { return unitsize_; }
    int channelCount() const { return nchans_; }

    void appendChunk(const QByteArray &bytes, qint64 firstSample,
                     qint64 nsamples);

    qint64 byteCount() const override { return data_.size(); }

    // Total appended sample count (derived from data size / unitsize).
    // May exceed sampleCount_ if appendChunk set firstSample beyond
    // the current tail; the gap is zero-filled conceptually.
    qint64 appendedSamples() const { return data_.size() / unitsize_; }

    // Raw byte access (for the painter / edge index). Caller must hold
    // the read lock for the duration of any pointer use.
    const QByteArray &rawBytes() const { return data_; }

    // Edge index access. Built on first call after data changes.
    // Const because building is a lazy cache mutation.
    const EdgeIndex &edgeIndex() const;

    // Convenience edge queries for one bit, in sample units. Build the
    // index lazily (like edgeIndex()) and forward to it. -1 if none.
    // `bit` is the channel's bit position within the packed unit.
    qint64 nextEdge(int bit, qint64 after) const {
        return edgeIndex().nextEdge(bit, after);
    }
    qint64 prevEdge(int bit, qint64 before) const {
        return edgeIndex().prevEdge(bit, before);
    }
    qint64 nearestEdge(int bit, qint64 sample) const {
        return edgeIndex().nearestEdge(bit, sample);
    }

    // Read lock for concurrent paint during capture. The GUI thread
    // appends; the (future) paint thread reads. For v0.1 paint is on
    // the GUI thread too, but the lock is here for correctness.
    mutable QReadWriteLock lock;

signals:
    void dataChanged();

private:
    int unitsize_;
    int nchans_;
    QByteArray data_;
    mutable EdgeIndex edges_;
    mutable bool edgesDirty_ = true;
};

} // namespace openmso::data
