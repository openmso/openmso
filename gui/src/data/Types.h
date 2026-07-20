#pragma once

#include <QtGlobal>

namespace openmso::data {

enum class SignalKind {
    Analog,
    Logic,
};

// Raw device code type for analog samples. Matches OCP encoding.dtype.
enum class AnalogDType {
    Int8,
    UInt8,
    Int16,
    UInt16,
    Float32,
    Float64,
};

// Bytes per sample for a given dtype.
inline int bytesPerSample(AnalogDType dt)
{
    switch (dt) {
    case AnalogDType::Int8:
    case AnalogDType::UInt8:   return 1;
    case AnalogDType::Int16:
    case AnalogDType::UInt16:  return 2;
    case AnalogDType::Float32: return 4;
    case AnalogDType::Float64: return 8;
    }
    return 0;
}

// Decode a raw sample code at `ptr` to a scaled voltage/value.
inline double decodeSample(AnalogDType dt, const char *ptr,
                           double scale, double offset)
{
    qint64 i = 0;
    double f = 0.0;
    switch (dt) {
    case AnalogDType::Int8:    i = *reinterpret_cast<const qint8 *>(ptr); break;
    case AnalogDType::UInt8:   i = *reinterpret_cast<const quint8 *>(ptr); break;
    case AnalogDType::Int16:   i = *reinterpret_cast<const qint16 *>(ptr); break;
    case AnalogDType::UInt16:  i = *reinterpret_cast<const quint16 *>(ptr); break;
    case AnalogDType::Float32: f = *reinterpret_cast<const float *>(ptr); break;
    case AnalogDType::Float64: f = *reinterpret_cast<const double *>(ptr); break;
    }
    if (dt == AnalogDType::Float32 || dt == AnalogDType::Float64)
        return f * scale + offset;
    return double(i) * scale + offset;
}

} // namespace openmso::data
