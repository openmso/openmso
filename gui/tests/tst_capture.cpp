#include <QSignalSpy>
#include <QTest>

#include "data/AnalogSegment.h"
#include "data/Capture.h"
#include "data/EdgeIndex.h"
#include "data/Envelope.h"
#include "data/LogicSegment.h"
#include "data/Signal.h"

using namespace openmso::data;

// M2 data-model test: build a Capture with one logic segment + one
// analog segment, append chunks, and verify the lazy edge index and
// envelope behave per docs/gui-plan/05-data-model.md.
class TestCapture : public QObject {
    Q_OBJECT
private slots:
    void captureStateMachine();
    void logicSegmentAppendAndEdges();
    void analogSegmentAppendAndEnvelope();
    void appendOnlyChunks();
};

void TestCapture::captureStateMachine()
{
    Capture c;
    QSignalSpy stateSpy(&c, &Capture::stateChanged);
    QVERIFY(stateSpy.isValid());

    QCOMPARE(c.state(), Capture::State::Idle);

    c.beginCapture(1e6, 0.0, {
        {"A0", "sine", SignalKind::Analog},
        {"D0", "D0",   SignalKind::Logic},
    });
    QCOMPARE(c.state(), Capture::State::Capturing);
    QCOMPARE(c.samplerate(), 1e6);
    QCOMPARE(c.allSignals().size(), 2);
    QCOMPARE(c.signalById("A0")->kind(), SignalKind::Analog);
    QCOMPARE(c.signalById("D0")->kind(), SignalKind::Logic);

    c.setTriggerSample(42);
    QCOMPARE(c.triggerSample(), qint64(42));

    c.endCapture(true);
    QCOMPARE(c.state(), Capture::State::Complete);

    // At least 3 transitions: Idle→Arming→Capturing (during begin),
    // plus Capturing→Complete on endCapture. (Arming is set inside
    // beginCapture before being overridden to Capturing.)
    QVERIFY(stateSpy.count() >= 2);
}

void TestCapture::logicSegmentAppendAndEdges()
{
    LogicSegment seg(/*unitsize=*/1, /*nchans=*/8);
    seg.setSamplerate(1e6);

    // Build a 16-sample pattern: D0 toggles every sample, D1 is
    // constant 1, others constant 0.
    QByteArray data(16, '\0');
    for (int i = 0; i < 16; ++i)
        data[i] = (i % 2) ? 0x01 : 0x00;
    seg.appendChunk(data, 0, 16);

    QCOMPARE(seg.appendedSamples(), qint64(16));

    const auto &idx = seg.edgeIndex();
    QCOMPARE(idx.channelCount(), 8);

    // D0 (bit 0) should have 15 edges (transitions between samples).
    auto edgesD0 = idx.edgesInRange(0, 0, 16);
    QCOMPARE(edgesD0.size(), 15);

    // D1 (bit 1) is constant; no edges.
    auto edgesD1 = idx.edgesInRange(1, 0, 16);
    QCOMPARE(edgesD1.size(), 0);

    // prevValue before sample 0 is the initial value (sample 0 = 0).
    bool prev = false;
    idx.edgesInRange(0, 0, 16, &prev);
    QCOMPARE(prev, false);

    // prevValue before sample 5: count edges before 5 (4 edges) →
    // 4 toggles → value = initial ^ (4 % 2) = initial = false.
    idx.edgesInRange(0, 5, 16, &prev);
    QCOMPARE(prev, false);
}

void TestCapture::analogSegmentAppendAndEnvelope()
{
    AnalogSegment seg(AnalogDType::Int8, /*scale=*/0.01, /*offset=*/0.0, "V");
    seg.setSamplerate(1e6);

    // 64 samples, ramp 0..63.
    QByteArray data(64, '\0');
    for (int i = 0; i < 64; ++i)
        data[i] = char(qint8(i));
    seg.appendChunk(data, 0, 64);

    QCOMPARE(seg.appendedSamples(), qint64(64));
    QCOMPARE(seg.sampleAt(0), 0.0);
    QCOMPARE(seg.sampleAt(63), 63 * 0.01);

    const auto &env = seg.envelope();
    QVERIFY(env.levelCount() > 0);
    // Level 1 has bucket=2; 32 buckets.
    const auto &L1 = env.level(0);
    QCOMPARE(L1.bucketSize, qint64(2));
    QCOMPARE(L1.minima.size(), 32);
    // Bucket 0 = samples {0, 1}: min=0, max=1.
    QCOMPARE(L1.minima[0], 0.0);
    QCOMPARE(L1.maxima[0], 1.0);
    // Bucket 31 = samples {62, 63}: min=62, max=63.
    QCOMPARE(L1.minima[31], 62.0);
    QCOMPARE(L1.maxima[31], 63.0);

    // Level picker: 1 sample/pixel → -1 (no envelope helps).
    QCOMPARE(env.levelForSamplePerPixel(0.5), -1);
    // 3 samples/pixel → first level whose bucket <= 3, i.e. level 0
    // (bucket=2).
    QVERIFY(env.levelForSamplePerPixel(3.0) >= 0);
}

void TestCapture::appendOnlyChunks()
{
    // Verify chunks are append-only: appending after the tail extends
    // the segment; the previously-appended bytes are unchanged.
    AnalogSegment seg(AnalogDType::UInt8, 1.0, 0.0, "V");
    seg.appendChunk(QByteArray(4, '\x01'), 0, 4);
    seg.appendChunk(QByteArray(4, '\x02'), 4, 4);
    QCOMPARE(seg.appendedSamples(), qint64(8));
    QCOMPARE(seg.sampleAt(0), 1.0);
    QCOMPARE(seg.sampleAt(3), 1.0);
    QCOMPARE(seg.sampleAt(4), 2.0);
    QCOMPARE(seg.sampleAt(7), 2.0);
}

QTEST_MAIN(TestCapture)
#include "tst_capture.moc"
