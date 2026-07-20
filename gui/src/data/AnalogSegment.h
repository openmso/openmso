#pragma once

#include <QByteArray>
#include <QReadWriteLock>

#include "Envelope.h"
#include "Segment.h"
#include "Types.h"

namespace openmso::data {

// Raw device codes for an analog channel. value = raw*scale + offset,
// applied only at paint/export time. Per 05-data-model.md.
class AnalogSegment : public Segment {
    Q_OBJECT
public:
    AnalogSegment(AnalogDType dt, double scale, double offset,
                  QString unit, QObject *parent = nullptr);

    AnalogDType dtype() const { return dtype_; }
    double scale() const { return scale_; }
    double offset() const { return offset_; }
    const QString &unit() const { return unit_; }
    int bytesPerSample() const { return openmso::data::bytesPerSample(dtype_); }

    void appendChunk(const QByteArray &bytes, qint64 firstSample,
                     qint64 nsamples);

    qint64 byteCount() const override { return data_.size(); }
    qint64 appendedSamples() const {
        return bytesPerSample() ? data_.size() / bytesPerSample() : 0;
    }

    const QByteArray &rawBytes() const { return data_; }

    // Lazy envelope. Const because building is a lazy cache mutation.
    const Envelope &envelope() const;

    // Decode one sample at `sample` to a scaled value.
    double sampleAt(qint64 sample) const;

    mutable QReadWriteLock lock;

signals:
    void dataChanged();

private:
    AnalogDType dtype_;
    double scale_;
    double offset_;
    QString unit_;
    QByteArray data_;
    mutable Envelope envelope_;
    mutable bool envelopeDirty_ = true;
};

} // namespace openmso::data
