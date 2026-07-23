#pragma once

#include <QString>
#include <QtGlobal>

namespace openmso::data {
class AnalogSegment;
class LogicSegment;
}

// Pure measurement engine: stats over a segment and sample range. No view
// or Qt-Widgets dependency, so it's cheap to unit-test in isolation and
// reusable (measurement dock now; decode/export/derived channels later).
// Per docs/gui-plan/11-milestones.md (measurements) — the analog stats
// also serve the M8 analog→logic threshold work.
namespace openmso::measure {

// Automatic parameters for an analog channel over a sample window.
struct AnalogStats {
    bool valid = false;
    qint64 sampleCount = 0;
    double min = 0.0;
    double max = 0.0;
    double pp = 0.0;    // peak-to-peak (max - min)
    double mean = 0.0;
    double rms = 0.0;
    QString unit;       // the channel's value unit (e.g. "V")
};

// Automatic parameters for one logic channel (bit) over a sample window.
// Timing fields are populated only when the window holds at least two
// rising edges (`hasTiming`); otherwise frequency/period/duty stay 0.
struct LogicStats {
    bool valid = false;
    qint64 edgeCount = 0;
    bool hasTiming = false;
    double frequency = 0.0;    // Hz, averaged over the window
    double period = 0.0;       // s
    double dutyCycle = 0.0;    // 0..1 (high time / period)
    double posWidthMin = 0.0;  // s, shortest high pulse in the window
    double posWidthMax = 0.0;  // s, longest high pulse in the window
};

// Measure an analog channel over sample range [first, last). The range is
// clamped to the data actually present; an empty range yields
// `valid == false`.
AnalogStats measureAnalog(const data::AnalogSegment &seg,
                          qint64 first, qint64 last);

// Measure logic channel `bit` over [first, last) at `samplerate` Hz.
LogicStats measureLogic(const data::LogicSegment &seg, int bit,
                        qint64 first, qint64 last, double samplerate);

} // namespace openmso::measure
