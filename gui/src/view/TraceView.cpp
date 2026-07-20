#include "TraceView.h"

#include "AnalogSignalTrace.h"
#include "Header.h"
#include "LogicSignalTrace.h"
#include "Ruler.h"
#include "Viewport.h"

#include "data/Capture.h"
#include "data/Signal.h"

#include <QGridLayout>
#include <QSplitter>
#include <QVBoxLayout>

namespace openmso::view {

TraceView::TraceView(QWidget *parent)
    : QFrame(parent)
{
    setFrameStyle(QFrame::NoFrame);

    // Layout:
    //   ┌──────┬─────────┐
    //   │      │  Ruler  │
    //   │ Head ├─────────┤
    //   │      │ Viewport│
    //   └──────┴─────────┘
    auto *grid = new QGridLayout(this);
    grid->setContentsMargins(0, 0, 0, 0);
    grid->setSpacing(0);

    header_ = new Header(this);
    ruler_ = new Ruler(this);
    viewport_ = new Viewport(this);

    grid->addWidget(header_, 0, 0, 2, 1);
    grid->addWidget(ruler_, 0, 1);
    grid->addWidget(viewport_, 1, 1);
    grid->setColumnStretch(1, 1);
    grid->setRowStretch(1, 1);

    connect(viewport_, &Viewport::stateChanged,
            this, &TraceView::onStateChanged);
    connect(viewport_, &Viewport::cursorMoved,
            this, &TraceView::cursorMoved);
}

void TraceView::setCapture(data::Capture *cap)
{
    if (capture_) {
        disconnect(capture_, nullptr, this, nullptr);
    }
    capture_ = cap;
    if (capture_) {
        connect(capture_, &data::Capture::captureBegan, this, [this]{ rebuildTraces(); });
        connect(capture_, &data::Capture::captureEnded, this, [this]{ rebuildTraces(); });
    }
    rebuildTraces();
}

void TraceView::rebuildTraces()
{
    traces_.clear();
    QList<Trace*> list;
    if (capture_) {
        const auto sigs = capture_->allSignals();
        for (auto *sig : sigs) {
            if (!sig) continue;
            if (sig->kind() == data::SignalKind::Analog) {
                auto *t = new AnalogSignalTrace(sig, this);
                list.append(t);
            } else {
                // One trace per bit in the logic segment.
                auto *seg = qobject_cast<data::LogicSegment *>(
                    sig->primarySegment());
                int nbits = seg ? seg->channelCount() : 0;
                if (nbits == 0) {
                    // No segment yet (pre-capture). Create a single
                    // placeholder trace so the channel shows up.
                    auto *t = new LogicSignalTrace(sig, 0, this);
                    list.append(t);
                } else {
                    for (int b = 0; b < nbits; ++b) {
                        auto *t = new LogicSignalTrace(sig, b, this);
                        list.append(t);
                    }
                }
            }
        }
    }
    // Assign curated colors round-robin.
    static const QColor palette[] = {
        QColor("#5B9BD5"), QColor("#ED7D31"), QColor("#A5A5A5"),
        QColor("#FFC000"), QColor("#4472C4"), QColor("#70AD47"),
        QColor("#264478"), QColor("#9B59B6"),
    };
    for (int i = 0; i < list.size(); ++i) {
        list[i]->setColor(palette[i % 8]);
        if (auto *st = qobject_cast<SignalTrace *>(list[i])) {
            if (st->signal()) st->signal()->setColor(palette[i % 8]);
        }
    }
    for (auto *t : list) traces_.append(t);
    viewport_->setTraces(list);
    header_->setTraces(list);
    update();
}

void TraceView::onStateChanged()
{
    header_->setState(state_);
    ruler_->setState(state_);
    // Viewport already updated itself; it owns `state_` though, so
    // mirror its state into our copy for external queries.
    state_ = viewport_->state();
}

void TraceView::onDataChanged()
{
    viewport_->update();
}

} // namespace openmso::view
