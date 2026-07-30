// Drives the real GUI against connected fx2lafw hardware: selects
// generic-fx2, connects, captures, and reports which channels toggle.

#include <QAction>
#include <QApplication>
#include <QComboBox>
#include <QImage>
#include <QTimer>

#include "data/Capture.h"
#include "data/LogicSegment.h"
#include "data/Signal.h"
#include "ui/MainWindow.h"
#include "ui/Session.h"
#include "view/TraceView.h"

#include <cstdio>

using namespace openmso;

namespace {

void trigger(QWidget *w, const QString &text)
{
    for (auto *a : w->findChildren<QAction *>()) {
        if (a->text() == text) {
            a->trigger();
            return;
        }
    }
    std::fprintf(stderr, "no action %s\n", qPrintable(text));
}

/// Edge count and mean period of one bit, in samples.
bool measure(const data::LogicSegment *segment, int bit, double *period, int *edges)
{
    QReadLocker locker(&segment->lock);
    const QByteArray &bytes = segment->rawBytes();
    const int unitsize = segment->unitsize();
    const qint64 samples = bytes.size() / unitsize;
    if (samples < 2)
        return false;

    QList<qint64> transitions;
    int previous = (bytes[0] >> bit) & 1;
    for (qint64 i = 1; i < samples; ++i) {
        const int value = (bytes[i * unitsize] >> bit) & 1;
        if (value != previous)
            transitions.append(i);
        previous = value;
    }
    *edges = transitions.size();
    if (transitions.size() < 3)
        return false;

    double total = 0;
    for (int i = 0; i + 2 < transitions.size(); ++i)
        total += transitions[i + 2] - transitions[i];
    *period = total / (transitions.size() - 2);
    return true;
}

} // namespace

int main(int argc, char *argv[])
{
    QApplication app(argc, argv);

    ui::MainWindow w;
    w.show();

    auto *picker = w.findChild<QComboBox *>();
    if (!picker) {
        std::fprintf(stderr, "FAIL: no plugin picker\n");
        return 1;
    }
    const int index = picker->findData(QStringLiteral("generic-fx2"));
    if (index < 0) {
        std::fprintf(stderr, "FAIL: generic-fx2 not in the plugin list\n");
        return 1;
    }
    picker->setCurrentIndex(index);

    QTimer::singleShot(100, [&] { trigger(&w, QStringLiteral("Connect")); });
    QTimer::singleShot(3000, [&] { trigger(&w, QStringLiteral("Start")); });

    QTimer::singleShot(12000, [&] {
        auto *session = w.findChild<ui::Session *>();
        auto *capture = session ? session->capture() : nullptr;
        if (!capture) {
            std::fprintf(stderr, "FAIL: no capture\n");
            app.exit(1);
            return;
        }
        std::printf("state=%d signals=%lld samplerate=%.0f\n", int(capture->state()),
                    static_cast<long long>(capture->allSignals().size()),
                    capture->samplerate());
        if (capture->state() != data::Capture::State::Complete) {
            std::fprintf(stderr, "FAIL: capture did not complete\n");
            app.exit(1);
            return;
        }

        int active = 0;
        for (auto *signal : capture->allSignals()) {
            auto *segment = qobject_cast<data::LogicSegment *>(signal->primarySegment());
            if (!segment) {
                std::fprintf(stderr, "FAIL: %s has no logic segment\n",
                             qPrintable(signal->id()));
                app.exit(1);
                return;
            }
            const int bit = signal->id().mid(1).toInt();
            double period = 0;
            int edges = 0;
            if (measure(segment, bit, &period, &edges)) {
                ++active;
                std::printf("  %s: %d edges, period %.1f samples = %.1f Hz\n",
                            qPrintable(signal->id()), edges, period,
                            capture->samplerate() / period);
            } else {
                std::printf("  %s: static (%lld samples)\n", qPrintable(signal->id()),
                            static_cast<long long>(segment->appendedSamples()));
            }
        }

        auto *view = w.findChild<view::TraceView *>();
        view->resize(1000, 600);
        // 5 ms across the viewport, so a 1 kHz wave reads as a square wave
        // rather than as a solid block of edges.
        view->state()->setScaleOffset(0.005 / view->width(), 0.0);
        QImage image(view->size(), QImage::Format_ARGB32_Premultiplied);
        image.fill(Qt::black);
        view->render(&image);
        int drawn = 0;
        for (int y = 0; y < image.height(); ++y)
            for (int x = 0; x < image.width(); ++x)
                if (image.pixel(x, y) != qRgb(0, 0, 0))
                    ++drawn;
        image.save(QStringLiteral("/tmp/fx2_bench.png"));

        std::printf("PASS: %d channel(s) toggling, %d non-bg pixels\n", active, drawn);
        app.exit(active > 0 && drawn > 1000 ? 0 : 1);
    });

    return app.exec();
}
