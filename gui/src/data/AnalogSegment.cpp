#include "AnalogSegment.h"

#include <algorithm>

namespace openmso::data {

AnalogSegment::AnalogSegment(AnalogDType dt, double scale, double offset,
                             QString unit, QObject *parent)
    : Segment(parent), dtype_(dt), scale_(scale), offset_(offset),
      unit_(std::move(unit)) {}

void AnalogSegment::appendChunk(const QByteArray &bytes, qint64 firstSample,
                                qint64 nsamples)
{
    QWriteLocker l(&lock);
    int bps = bytesPerSample();
    qint64 tail = appendedSamples();
    if (firstSample > tail) {
        QByteArray pad(int((firstSample - tail) * bps), '\0');
        data_.append(pad);
    }
    data_.append(bytes);
    sampleCount_ = std::max(sampleCount_, firstSample + nsamples);
    envelopeDirty_ = true;
    emit dataChanged();
}

const Envelope &AnalogSegment::envelope() const
{
    if (envelopeDirty_) {
        QWriteLocker l(&lock);
        envelope_.build(data_, bytesPerSample(), dtype_, appendedSamples());
        envelopeDirty_ = false;
    }
    return envelope_;
}

double AnalogSegment::sampleAt(qint64 sample) const
{
    QReadLocker l(&lock);
    int bps = bytesPerSample();
    if (sample < 0 || sample * bps + bps > data_.size())
        return 0.0;
    return decodeSample(dtype_, data_.constData() + sample * bps,
                        scale_, offset_);
}

} // namespace openmso::data
