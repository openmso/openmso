#include <QImage>
#include <QPainter>
#include <QSignalSpy>
#include <QTest>

#include "data/AnalogSegment.h"
#include "data/Capture.h"
#include "data/LogicSegment.h"
#include "data/Signal.h"
#include "view/AnalogSignalTrace.h"
#include "view/LogicSignalTrace.h"
#include "view/Ruler.h"
#include "view/TraceView.h"
#include "view/Viewport.h"

using namespace openmso::data;
using namespace openmso::view;

// M2 view test: build a capture with synthetic data, point a TraceView
// at it, and paint offscreen. Verifies no crashes and that paint
// produces a non-empty image. Visual correctness is a manual check.
class TestView : public QObject {
    Q_OBJECT
private slots:
    void paintWithCapture();
    void paintEmptyCapture();
};

namespace {

// Build a capture with one logic signal (8 bits, 256 samples, counter
// pattern) and one analog signal (256 samples, sine). Same shape as
// the demo plugin's output but synthetic.
Capture *makeCapture(QObject *parent)
{
    auto *cap = new Capture(parent);
    cap->beginCapture(1e6, 0.0, {
        {"A0", "sine", SignalKind::Analog},
    });
    // Add a logic signal manually (capture.begin cleared it).
    auto *logicSig = new Signal("D0", "D0", SignalKind::Logic, cap);

    // Analog: 256 samples, sine.
    auto *aSeg = new AnalogSegment(AnalogDType::Int8, 0.01, 0.0, "V", logicSig);
    aSeg->setSamplerate(1e6);
    QByteArray abytes(256, '\0');
    for (int i = 0; i < 256; ++i)
        abytes[i] = char(qint8(50 * std::sin(2 * M_PI * i / 256)));
    aSeg->appendChunk(abytes, 0, 256);
    // Append aSeg to its own signal — but we created it under logicSig
    // by accident. Re-parent: create a proper analog signal.
    auto *analogSig = new Signal("A0", "sine", SignalKind::Analog, cap);
    auto *aSeg2 = new AnalogSegment(AnalogDType::Int8, 0.01, 0.0, "V", analogSig);
    aSeg2->setSamplerate(1e6);
    aSeg2->appendChunk(abytes, 0, 256);
    analogSig->appendSegment(aSeg2);
    // The capture's signal list was set in beginCapture; replace D0
    // with our logicSig. (Direct manipulation for the test.)
    auto sigs = cap->allSignals();
    for (auto *s : sigs) {
        if (s->id() == "A0") {
            // Move the segment over.
            auto *old = s->primarySegment();
            analogSig->clearSegments();
            // Already has aSeg2; nothing to do.
        }
    }
    // Append logic segment.
    auto *lSeg = new LogicSegment(1, 8, logicSig);
    lSeg->setSamplerate(1e6);
    QByteArray lbytes(256, '\0');
    for (int i = 0; i < 256; ++i)
        lbytes[i] = char(i & 0x7F);
    lSeg->appendChunk(lbytes, 0, 256);
    logicSig->appendSegment(lSeg);

    // Note: cap->signals() reflects the beginCapture list; for the
    // view test we don't rely on it — we build traces directly.
    return cap;
}

} // namespace

void TestView::paintWithCapture()
{
    Capture cap;
    cap.beginCapture(1e6, 0.0, {
        {"A0", "sine", SignalKind::Analog},
    });

    // Append real segments to the signals created by beginCapture.
    auto *analogSig = cap.signalById("A0");
    QVERIFY(analogSig);
    auto *aSeg = new AnalogSegment(AnalogDType::Int8, 0.01, 0.0, "V", analogSig);
    aSeg->setSamplerate(1e6);
    QByteArray abytes(256, '\0');
    for (int i = 0; i < 256; ++i)
        abytes[i] = char(qint8(50 * std::sin(2 * M_PI * i / 256)));
    aSeg->appendChunk(abytes, 0, 256);
    analogSig->appendSegment(aSeg);

    TraceView tv;
    tv.setCapture(&cap);
    tv.resize(400, 300);

    // Render to an image offscreen.
    QImage img(tv.size(), QImage::Format_ARGB32_Premultiplied);
    img.fill(Qt::black);
    QVERIFY(!img.isNull());
    tv.render(&img);

    // The image should have some non-background pixels in the
    // viewport area (any trace pixel drawn).
    int drawn = 0;
    for (int y = 0; y < img.height(); ++y)
        for (int x = 0; x < img.width(); ++x)
            if (img.pixel(x, y) != qRgb(0, 0, 0))
                ++drawn;
    QVERIFY(drawn > 0);
}

void TestView::paintEmptyCapture()
{
    TraceView tv;
    tv.setCapture(nullptr);
    tv.resize(200, 100);
    QImage img(tv.size(), QImage::Format_ARGB32_Premultiplied);
    img.fill(Qt::black);
    tv.render(&img);  // should not crash
    QVERIFY(!img.isNull());
}

QTEST_MAIN(TestView)
#include "tst_view.moc"
