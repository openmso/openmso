#pragma once

#include <QVector>
#include <QtGlobal>

#include "data/Types.h"

namespace openmso::data {

// Min/max envelope pyramid for analog segments. Each level halves the
// resolution: level L bucket = 2^L samples. The painter picks the
// level where samples_per_pixel ≈ bucket_size and draws one
// vertical bar (min..max) per pixel column.
//
// Built lazily on first paint after data changes. Memory is O(nsamples)
// across all levels (geometric series).
class Envelope {
public:
    struct Level {
        qint64 bucketSize = 1;     // samples per bucket
        // Parallel arrays of min/max raw codes (NOT scaled).
        QVector<double> minima;
        QVector<double> maxima;
    };

    void build(const QByteArray &data, int bytesPerSample,
               AnalogDType dtype, qint64 nsamples);

    int levelCount() const { return levels_.size(); }
    const Level &level(int i) const { return levels_[i]; }

    // Pick the pyramid level whose bucket size is >= samplesPerPixel.
    // Returns -1 if even level 0 (1 sample/bucket) is too coarse
    // (caller should draw per-sample polyline).
    int levelForSamplePerPixel(double samplesPerPixel) const;

private:
    QVector<Level> levels_;
};

} // namespace openmso::data
