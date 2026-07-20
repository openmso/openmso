#pragma once

#include <QObject>

namespace openmso::data {

// Base for a contiguous run of samples. Per 05-data-model.md.
// Subclasses: LogicSegment (bit-packed), AnalogSegment (raw codes).
class Segment : public QObject {
    Q_OBJECT
public:
    Segment(QObject *parent = nullptr);

    qint64 sampleCount() const { return sampleCount_; }
    double samplerate() const { return samplerate_; }
    void setSamplerate(double sr) { samplerate_ = sr; }

    // Total bytes appended so far (raw payload bytes from capture.data).
    virtual qint64 byteCount() const = 0;

    // Subclasses emit dataChanged when a chunk is appended. The
    // lazy edge index / envelope is invalidated on append.

protected:
    qint64 sampleCount_ = 0;
    double samplerate_ = 0.0;
};

} // namespace openmso::data
