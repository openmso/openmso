#pragma once

#include <QByteArray>

#include "data/Types.h"

namespace openmso::measure {

// Parameters for deriving a logic channel from an analog one with a
// dual-threshold Schmitt trigger. Rising past `vHigh` (Vr) sets the output
// high; falling past `vLow` (Vf) sets it low. Keeping vLow < vHigh gives
// hysteresis, so noise around a single crossing level doesn't produce a
// burst of edges. Per docs/gui-plan HANDOFF (analog->logic derived channel).
struct SchmittParams {
    double vHigh = 0.0;          // Vr — rising threshold (volts).
    double vLow = 0.0;           // Vf — falling threshold (volts).
    bool invert = false;         // flip the output level.
    qint64 deglitchSamples = 0;  // drop runs shorter than this (both levels).
};

// Run the Schmitt trigger over a *snapshot* of analog samples, producing
// one packed logic byte per sample (bit 0 = level, value 0 or 1) suitable
// for a unitsize-1 LogicSegment.
//
// This is a pure function of the snapshot — it holds no lock and touches no
// live segment, so it is safe to call on a worker thread. Callers take the
// segment's read lock, copy out the raw bytes (cheap, copy-on-write) and
// the decode params, release the lock, then call this. See
// view::DerivedChannel, which runs it off the GUI thread.
QByteArray schmittWalk(const QByteArray &raw, data::AnalogDType dtype,
                       double scale, double offset, qint64 nsamples,
                       const SchmittParams &p);

} // namespace openmso::measure
