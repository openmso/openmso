#include "LogicSegment.h"

namespace openmso::data {

LogicSegment::LogicSegment(int unitsize, int nchans, QObject *parent)
    : Segment(parent), unitsize_(unitsize), nchans_(nchans)
{
    Q_ASSERT(unitsize >= 1);
    Q_ASSERT(nchans >= 1);
}

void LogicSegment::appendChunk(const QByteArray &bytes, qint64 firstSample,
                               qint64 nsamples)
{
    QWriteLocker l(&lock);
    // If there's a gap between current tail and firstSample, pad with
    // zeros (last-known state held). In practice the demo plugin sends
    // contiguous chunks, but defensive handling matters for real
    // hardware with gaps.
    qint64 tail = appendedSamples();
    if (firstSample > tail) {
        QByteArray pad(int((firstSample - tail) * unitsize_), '\0');
        data_.append(pad);
    }
    data_.append(bytes);
    sampleCount_ = std::max(sampleCount_, firstSample + nsamples);
    edgesDirty_ = true;
    emit dataChanged();
}

const EdgeIndex &LogicSegment::edgeIndex() const
{
    if (edgesDirty_) {
        QWriteLocker l(&lock);
        edges_.build(data_, unitsize_, nchans_, appendedSamples());
        edgesDirty_ = false;
    }
    return edges_;
}

} // namespace openmso::data
