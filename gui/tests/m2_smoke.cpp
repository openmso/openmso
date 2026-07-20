// Manual smoke test for M2: launch the GUI against the demo plugin,
// programmatically click Connect → Start, wait for capture.end, then
// render the TraceView to an image and assert non-trivial pixels were
// drawn. Not a QTest — we want a main() that exercises the real
// MainWindow path.
//
// Usage: ./m2_smoke
// Exits 0 on success, non-zero on failure.

#include <QApplication>
#include <QImage>
#include <QTimer>

#include "ui/MainWindow.h"
#include "ui/Session.h"
#include "data/Capture.h"
#include "data/Signal.h"
#include "data/LogicSegment.h"
#include "data/AnalogSegment.h"
#include "view/TraceView.h"

#include <cstdio>

using namespace openmso;

int main(int argc, char *argv[])
{
    QApplication app(argc, argv);
    app.setApplicationName("m2_smoke");

    ui::MainWindow w;
    w.show();

    // Click Connect (plugin picker defaults to "demo").
    QTimer::singleShot(100, [&]{
        // Find the Connect action and trigger it.
        for (auto *a : w.findChildren<QAction*>()) {
            if (a->text() == "Connect") {
                a->trigger();
                break;
            }
        }
    });

    // Click Start shortly after.
    QTimer::singleShot(500, [&]{
        for (auto *a : w.findChildren<QAction*>()) {
            if (a->text() == "Start") {
                a->trigger();
                break;
            }
        }
    });

    // Wait for capture completion (demo takes <1s), then verify and quit.
    QTimer::singleShot(2500, [&]{
        auto *session = w.findChild<ui::Session*>();
        if (!session) {
            std::fprintf(stderr, "FAIL: no session\n");
            app.exit(1);
            return;
        }
        auto *cap = session->capture();
        if (!cap) {
            std::fprintf(stderr, "FAIL: no capture\n");
            app.exit(1);
            return;
        }
        if (cap->state() != data::Capture::State::Complete) {
            std::fprintf(stderr, "FAIL: capture state = %d (expected Complete)\n",
                         int(cap->state()));
            app.exit(1);
            return;
        }
        if (cap->allSignals().size() != 10) {
            std::fprintf(stderr, "FAIL: expected 10 signals, got %d\n",
                         cap->allSignals().size());
            app.exit(1);
            return;
        }

        // Verify each signal has a segment with data.
        for (auto *sig : cap->allSignals()) {
            auto *seg = sig->primarySegment();
            if (!seg || seg->byteCount() == 0) {
                std::fprintf(stderr, "FAIL: signal %s has no data\n",
                             qPrintable(sig->id()));
                app.exit(1);
                return;
            }
        }

        // Render the TraceView to an image and verify non-trivial output.
        auto *tv = w.findChild<view::TraceView*>();
        if (!tv) {
            std::fprintf(stderr, "FAIL: no TraceView\n");
            app.exit(1);
            return;
        }
        tv->resize(800, 600);
        QImage img(tv->size(), QImage::Format_ARGB32_Premultiplied);
        img.fill(Qt::black);
        tv->render(&img);
        int drawn = 0;
        for (int y = 0; y < img.height(); ++y)
            for (int x = 0; x < img.width(); ++x)
                if (img.pixel(x, y) != qRgb(0, 0, 0)) ++drawn;
        if (drawn < 1000) {
            std::fprintf(stderr, "FAIL: only %d non-bg pixels rendered\n", drawn);
            app.exit(1);
            return;
        }
        std::printf("PASS: %d signals, %d non-bg pixels, state=Complete\n",
                    cap->allSignals().size(), drawn);
        img.save(QStringLiteral("/tmp/m2_smoke.png"));
        app.exit(0);
    });

    return app.exec();
}
