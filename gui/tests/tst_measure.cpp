#include <QTest>

#include "data/AnalogSegment.h"
#include "data/LogicSegment.h"
#include "measure/Measure.h"

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

QTEST_MAIN(TestMeasure)
#include "tst_measure.moc"
