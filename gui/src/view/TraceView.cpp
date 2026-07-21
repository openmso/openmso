#include "TraceView.h"

#include "AnalogSignalTrace.h"
#include "Header.h"
#include "LogicSignalTrace.h"
#include "Ruler.h"
#include "Viewport.h"

#include "data/Capture.h"
#include "data/Signal.h"
#include "util/ChannelColors.h"

#include <QGridLayout>
#include <QResizeEvent>
#include <QScrollBar>

#include <algorithm>

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
    vscroll_ = new QScrollBar(Qt::Vertical, this);

    //   ┌──────┬─────────┬──┐
    //   │      │  Ruler  │  │
    //   │ Head ├─────────┤vb│
    //   │      │ Viewport│  │
    //   └──────┴─────────┴──┘
    grid->addWidget(header_, 0, 0, 2, 1);
    grid->addWidget(ruler_, 0, 1);
    grid->addWidget(viewport_, 1, 1);
    grid->addWidget(vscroll_, 1, 2);
    grid->setColumnStretch(1, 1);
    grid->setRowStretch(1, 1);

    connect(viewport_, &Viewport::stateChanged,
            this, &TraceView::onStateChanged);
    connect(viewport_, &Viewport::cursorMoved,
            this, &TraceView::cursorMoved);
    connect(vscroll_, &QScrollBar::valueChanged, this, [this](int v) {
        viewport_->state().yOffset = v;
        viewport_->update();
        header_->setState(viewport_->state());
    });
}

void TraceView::setCapture(data::Capture *cap)
{
    if (capture_) {
        disconnect(capture_, nullptr, this, nullptr);
    }
    capture_ = cap;
    if (capture_) {
        connect(capture_, &data::Capture::channelsChanged, this, [this]{ rebuildTraces(); });
        connect(capture_, &data::Capture::captureEnded, this, [this]{
            rebuildTraces();
            fitToData();
        });
    }
    rebuildTraces();
}

void TraceView::rebuildTraces()
{
    traces_.clear();
    QList<Trace*> list;

    // Colors follow the platform theme (dark is primary). Ownership of
    // the color lives on data::Signal; the trace mirrors it for paint.
    const util::Theme theme = util::themeFor(palette());

    if (capture_) {
        const auto sigs = capture_->allSignals();
        for (auto *sig : sigs) {
            if (!sig) continue;
            Trace *t = nullptr;
            if (sig->kind() == data::SignalKind::Analog) {
                sig->setColor(util::analogColor(sig->channelIndex(), theme));
                t = new AnalogSignalTrace(sig, this);
            } else {
                // Exactly one trace per logic channel. The channel's
                // ordinal is its bit position within the packed segment
                // unit and its resistor-code color index.
                const int bit = std::max(0, sig->channelIndex());
                sig->setColor(util::logicColor(sig->channelIndex(), theme));
                t = new LogicSignalTrace(sig, bit, this);
            }
            t->setColor(sig->color());
            list.append(t);
        }
    }

    for (auto *t : list) traces_.append(t);
    viewport_->setTraces(list);
    header_->setTraces(list);
    header_->setState(viewport_->state());
    syncScrollBar();
    update();
}

void TraceView::onStateChanged()
{
    // The Viewport owns the authoritative ViewState; mirror it into our
    // copy and push it to the widgets that observe it.
    state_ = viewport_->state();
    ruler_->setState(state_);
    header_->setState(state_);
    syncScrollBar();
}

void TraceView::syncScrollBar()
{
    const int maxY = std::max(0, viewport_->contentHeight() - viewport_->height());
    // Clamp the current offset into the new range before reflecting it.
    ViewState &st = viewport_->state();
    st.yOffset = std::max(0, std::min(maxY, st.yOffset));

    QSignalBlocker block(vscroll_);
    vscroll_->setRange(0, maxY);
    vscroll_->setPageStep(viewport_->height());
    vscroll_->setSingleStep(state_.rowHeight);
    vscroll_->setValue(st.yOffset);
    vscroll_->setVisible(maxY > 0);
}

void TraceView::fitToData()
{
    if (!capture_) return;
    const double sr = capture_->samplerate();
    const qint64 n = capture_->sampleCount();
    const int w = viewport_->width();
    if (sr <= 0 || n <= 0 || w <= 0) return;

    ViewState &st = viewport_->state();
    st.offset = 0.0;
    st.scale = (double(n) / sr) / double(w);
    onStateChanged();
    viewport_->update();
}

void TraceView::resizeEvent(QResizeEvent *e)
{
    QFrame::resizeEvent(e);
    syncScrollBar();
}

void TraceView::zoomIn()  { viewport_->zoom(0.5); }
void TraceView::zoomOut() { viewport_->zoom(2.0); }
void TraceView::toggleCursors() { viewport_->toggleCursors(); }

void TraceView::onDataChanged()
{
    viewport_->update();
}

} // namespace openmso::view
