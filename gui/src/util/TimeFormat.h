#pragma once

#include <QString>

namespace openmso::util {

// Format a time in seconds with an SI unit (s / ms / µs / ns) chosen
// from its magnitude. If `decimals` is negative the number of decimals
// is chosen automatically and trailing zeros are trimmed; otherwise the
// value is shown with exactly `decimals` places.
QString formatTime(double seconds, int decimals = -1);

// Format a duration (Δt) the same way; always shows a couple of
// significant decimals so small deltas stay readable.
QString formatDelta(double seconds);

// Snap a raw spacing to the next "nice" 1/2/5 × 10^n value ≥ rawStep.
// Used to pick ruler tick spacing.
double niceTickStep(double rawStep);

// Number of decimal places needed to distinguish adjacent ticks of the
// given `step`, when both are rendered in the unit that `formatTime`
// would pick for `referenceValue`.
int decimalsForStep(double step, double referenceValue);

} // namespace openmso::util
