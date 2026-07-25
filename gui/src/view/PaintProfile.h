#pragma once

#include <QtGlobal>

namespace openmso::view {

// Runtime paint profiling, gated by the OPENMSO_PROFILE_PAINT environment
// variable (set it to any non-empty value to enable). When on, the trace
// paint paths emit a qDebug line per lane per repaint carrying build/draw
// timings and the paint-surface geometry (size, devicePixelRatio).
//
// This is deliberately permanent, not scaffolding: paint cost is critical
// to this app and depends on runtime factors a rebuild can't easily
// reproduce — notably the monitor's (possibly fractional) devicePixelRatio,
// which turns some pens quadratic (see LogicSignalTrace). Being able to
// flip it on against a running instance — e.g. mid live-capture — without
// recompiling is the point.
//
// Cost when disabled: one relaxed load of a function-local static bool.
inline bool paintProfileEnabled()
{
    static const bool on = !qEnvironmentVariableIsEmpty("OPENMSO_PROFILE_PAINT");
    return on;
}

} // namespace openmso::view
