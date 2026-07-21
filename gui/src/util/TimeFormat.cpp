#include "TimeFormat.h"

#include <cmath>

namespace openmso::util {

namespace {

// Pick {divisor, suffix} for a magnitude of `a` seconds.
struct Unit { double scale; const char *suffix; };

Unit unitFor(double a)
{
    if (a >= 1.0)  return {1.0,   "s"};
    if (a >= 1e-3) return {1e3,   "ms"};
    if (a >= 1e-6) return {1e6,   "µs"};   // µs
    return {1e9, "ns"};
}

QString trimTrailingZeros(QString s)
{
    if (!s.contains('.')) return s;
    while (s.endsWith('0')) s.chop(1);
    if (s.endsWith('.')) s.chop(1);
    return s;
}

} // namespace

QString formatTime(double seconds, int decimals)
{
    const double a = std::abs(seconds);
    const Unit u = unitFor(a > 0 ? a : 1.0);
    const double v = seconds * u.scale;
    if (decimals < 0) {
        QString num = trimTrailingZeros(QString::number(v, 'f', 3));
        return QStringLiteral("%1 %2").arg(num, QString::fromUtf8(u.suffix));
    }
    return QStringLiteral("%1 %2")
        .arg(v, 0, 'f', decimals)
        .arg(QString::fromUtf8(u.suffix));
}

QString formatDelta(double seconds)
{
    const double a = std::abs(seconds);
    const Unit u = unitFor(a > 0 ? a : 1.0);
    return QStringLiteral("%1 %2")
        .arg(seconds * u.scale, 0, 'f', 3)
        .arg(QString::fromUtf8(u.suffix));
}

double niceTickStep(double rawStep)
{
    if (rawStep <= 0) return 1.0;
    const double mag = std::pow(10.0, std::floor(std::log10(rawStep)));
    const double norm = rawStep / mag;
    double step;
    if (norm <= 1.0)      step = 1.0;
    else if (norm <= 2.0) step = 2.0;
    else if (norm <= 5.0) step = 5.0;
    else                  step = 10.0;
    return step * mag;
}

int decimalsForStep(double step, double referenceValue)
{
    const Unit u = unitFor(std::abs(referenceValue) > 0
                               ? std::abs(referenceValue)
                               : std::abs(step));
    const double stepInUnit = step * u.scale;
    if (stepInUnit <= 0) return 0;
    // Enough decimals that the step is representable: e.g. step 0.5 → 1,
    // step 5 → 0, step 0.02 → 2. Clamp to a sane range.
    int d = int(std::ceil(-std::log10(stepInUnit)));
    if (d < 0) d = 0;
    if (d > 3) d = 3;
    return d;
}

} // namespace openmso::util
