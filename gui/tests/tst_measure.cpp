#include <QTest>

#include "data/AnalogSegment.h"
#include "data/LogicSegment.h"
#include "measure/Measure.h"
#include "measure/Schmitt.h"

#include <cmath>

using namespace openmso::data;
using namespace openmso::measure;

// Unit tests for the pure measurement engine.
class TestMeasure : public QObject {
    Q_OBJECT
private slots:
    void analogSquareStats();
    void analogEmptyRange();
    void logicSquareTiming();
    void logicNoTiming();
    void schmittHysteresisRejectsNoise();
    void schmittInvertAndDeglitch();
};

// A ±1.0 V square wave (raw ±100 codes, scale 0.01) over 100 samples:
// min=-1, max=+1, pp=2, mean≈0, rms=1.
void TestMeasure::analogSquareStats()
{
    AnalogSegment seg(AnalogDType::Int8, 0.01, 0.0, "V");
    seg.setSamplerate(1e6);
    QByteArray bytes(100, '\0');
    for (int i = 0; i < 100; ++i)
        bytes[i] = char(qint8((i % 2 == 0) ? 100 : -100));
    seg.appendChunk(bytes, 0, 100);

    const AnalogStats s = measureAnalog(seg, 0, 100);
    QVERIFY(s.valid);
    QCOMPARE(s.sampleCount, qint64(100));
    QVERIFY(std::abs(s.min - (-1.0)) < 1e-9);
    QVERIFY(std::abs(s.max - 1.0) < 1e-9);
    QVERIFY(std::abs(s.pp - 2.0) < 1e-9);
    QVERIFY(std::abs(s.mean) < 1e-9);
    QVERIFY(std::abs(s.rms - 1.0) < 1e-9);
    QCOMPARE(s.unit, QStringLiteral("V"));
}

void TestMeasure::analogEmptyRange()
{
    AnalogSegment seg(AnalogDType::Int8, 0.01, 0.0, "V");
    seg.setSamplerate(1e6);
    QByteArray bytes(10, '\0');
    seg.appendChunk(bytes, 0, 10);

    // Degenerate and out-of-range windows are invalid, not crashes.
    QVERIFY(!measureAnalog(seg, 5, 5).valid);
    QVERIFY(!measureAnalog(seg, 50, 60).valid);
}

// A 1-bit channel toggling every 10 samples over 100 samples at 1 MHz:
// edges at 10,20,...,90; rising at 10,30,50,70,90 ⇒ 50 kHz, 20 µs period,
// 50% duty, 10 µs high pulses.
void TestMeasure::logicSquareTiming()
{
    LogicSegment seg(1, 1);
    seg.setSamplerate(1e6);
    QByteArray bytes(100, '\0');
    for (int i = 0; i < 100; ++i)
        bytes[i] = char((i / 10) & 1);
    seg.appendChunk(bytes, 0, 100);

    const LogicStats s = measureLogic(seg, 0, 0, 100, 1e6);
    QVERIFY(s.valid);
    QVERIFY(s.hasTiming);
    QCOMPARE(s.edgeCount, qint64(9));
    QVERIFY(std::abs(s.frequency - 50000.0) < 1e-6);
    QVERIFY(std::abs(s.period - 2e-5) < 1e-12);
    QVERIFY(std::abs(s.dutyCycle - 0.5) < 1e-9);
    QVERIFY(std::abs(s.posWidthMin - 1e-5) < 1e-12);
    QVERIFY(std::abs(s.posWidthMax - 1e-5) < 1e-12);
}

// Too few edges for a period: valid stats, but no timing.
void TestMeasure::logicNoTiming()
{
    LogicSegment seg(1, 1);
    seg.setSamplerate(1e6);
    QByteArray bytes(100, '\0');
    for (int i = 50; i < 100; ++i)   // a single rising edge at sample 50.
        bytes[i] = char(1);
    seg.appendChunk(bytes, 0, 100);

    const LogicStats s = measureLogic(seg, 0, 0, 100, 1e6);
    QVERIFY(s.valid);
    QCOMPARE(s.edgeCount, qint64(1));
    QVERIFY(!s.hasTiming);
    QCOMPARE(s.frequency, 0.0);
}

// Decode a packed 1-bit logic byte stream to a level string for easy
// assertions ("00110" etc).
static QString bitsToString(const QByteArray &b)
{
    QString s;
    for (char c : b) s += (c & 1) ? '1' : '0';
    return s;
}

// Hysteresis: a signal that dithers around a single crossing level must not
// produce edges once thresholds straddle it. Ramp up through Vf..Vr region
// with noise; only a genuine excursion past Vr should latch high.
void TestMeasure::schmittHysteresisRejectsNoise()
{
    // Float samples, scale 1.0. Vr=0.7, Vf=0.3. Values wobble in the
    // 0.4..0.6 dead-band (between thresholds) after one clean rise, then
    // fall cleanly. The dead-band wobble must yield NO extra edges.
    const QVector<double> v = {
        0.0, 0.1, 0.2,            // low
        0.8, 0.9,                 // rise past Vr -> high
        0.6, 0.5, 0.55, 0.45, 0.6,// wobble in dead-band -> stays high
        0.2, 0.1,                 // fall past Vf -> low
        0.4, 0.5, 0.45,           // wobble in dead-band -> stays low
    };
    QByteArray raw(v.size() * int(sizeof(float)), Qt::Uninitialized);
    auto *f = reinterpret_cast<float *>(raw.data());
    for (int i = 0; i < v.size(); ++i) f[i] = float(v[i]);

    SchmittParams p;
    p.vHigh = 0.7; p.vLow = 0.3;
    const QByteArray bits = schmittWalk(raw, AnalogDType::Float32, 1.0, 0.0,
                                        v.size(), p);
    // low(3) high(2+5) low(2+3): exactly one rise and one fall.
    QCOMPARE(bitsToString(bits),
             QStringLiteral("000") + "1111111" + "00000");
}

// Invert flips levels; de-glitch absorbs a sub-threshold-width pulse into
// its predecessor.
void TestMeasure::schmittInvertAndDeglitch()
{
    // Clean square: 5 low, 5 high, 5 low (Int8, scale 1). Threshold at 50.
    QByteArray raw(15, '\0');
    for (int i = 5; i < 10; ++i) raw[i] = char(100);

    SchmittParams p;
    p.vHigh = 60; p.vLow = 40;

    // Plain.
    QCOMPARE(bitsToString(schmittWalk(raw, AnalogDType::Int8, 1.0, 0.0, 15, p)),
             QStringLiteral("000001111100000"));

    // Inverted.
    p.invert = true;
    QCOMPARE(bitsToString(schmittWalk(raw, AnalogDType::Int8, 1.0, 0.0, 15, p)),
             QStringLiteral("111110000011111"));

    // De-glitch a 5-sample high pulse with a 6-sample minimum: it's shorter
    // than the minimum, so it's absorbed into the preceding low run.
    p.invert = false;
    p.deglitchSamples = 6;
    QCOMPARE(bitsToString(schmittWalk(raw, AnalogDType::Int8, 1.0, 0.0, 15, p)),
             QStringLiteral("000000000000000"));
}

QTEST_MAIN(TestMeasure)
#include "tst_measure.moc"
