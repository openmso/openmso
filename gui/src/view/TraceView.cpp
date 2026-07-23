#include "TraceView.h"

#include "ChannelModel.h"
#include "Header.h"
#include "Ruler.h"
#include "SignalTrace.h"
#include "Viewport.h"

#include "data/Capture.h"
#include "data/Signal.h"
#include "util/ChannelColors.h"

#include <QGridLayout>
#include <QPalette>
#include <QResizeEvent>
#include <QScrollBar>
#include <QWidget>

#include <algorithm>

namespace openmso::view {

TraceView::TraceView(QWidget *parent)
    : QFrame(parent)
{
    setFrameStyle(QFrame::NoFrame);

    // The single source of truth for scale/offset/scroll/cursors. The
    // Viewport, Ruler and Header all hold this same pointer, mutate it
    // through its setters, and repaint when it emits changed().
    state_ = new ViewState(this);

    // The ordered, mutable channel list — single source of truth for row
    // order/membership, decoupled from the Capture. Header/Viewport read
    // it and repaint on changed().
    channels_ = new ChannelModel(this);

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
    hscroll_ = new QScrollBar(Qt::Horizontal, this);

    // Corner spacer in the ruler row above the header, so the header's
    // channel rows start level with the viewport (row 1) instead of the
    // ruler top — otherwise every label is offset up by the ruler height.
    auto *corner = new QWidget(this);
    corner->setAutoFillBackground(true);
    corner->setBackgroundRole(QPalette::Window);

    header_->setViewState(state_);
    ruler_->setViewState(state_);
    viewport_->setViewState(state_);
    header_->setChannelModel(channels_);
    viewport_->setChannelModel(channels_);

    // Row membership/order changed: drop a stale selection, resize the
    // vertical scroll range, repaint.
    connect(channels_, &ChannelModel::changed, this, [this] {
        if (state_->selectedRow() >= channels_->count())
            state_->setSelectedRow(-1);
        syncScrollBars();
        update();
    });

    //   ┌──────┬─────────┬──┐
    //   │corner│  Ruler  │  │
    //   ├──────┼─────────┤vb│
    //   │ Head │ Viewport│  │
    //   ├──────┼─────────┼──┤
    //   │      │ hscroll │  │
    //   └──────┴─────────┴──┘
    grid->addWidget(corner, 0, 0);
    grid->addWidget(header_, 1, 0);
    grid->addWidget(ruler_, 0, 1);
    grid->addWidget(viewport_, 1, 1);
    grid->addWidget(vscroll_, 1, 2);
    grid->addWidget(hscroll_, 2, 1);
    grid->setColumnStretch(1, 1);
    grid->setRowStretch(1, 1);

    // Observe the shared state: keep both scrollbars in sync and bubble
    // cursor moves up to the window's status bar.
    connect(state_, &ViewState::changed, this, &TraceView::syncScrollBars);
    connect(state_, &ViewState::cursorMoved, this, &TraceView::cursorMoved);
    // Vertical scrollbar drives the trace-stack scroll offset.
    connect(vscroll_, &QScrollBar::valueChanged, this, [this](int v) {
        state_->setYOffset(v);
    });
    // Horizontal scrollbar drives the time offset. Its value is in
    // content pixels from the data start; convert back to seconds.
    connect(hscroll_, &QScrollBar::valueChanged, this, [this](int v) {
        state_->setOffset(state_->dataStart() + v * state_->scale());
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
            emit dataChanged();
        });
        // Live data extends the clamp bounds (and needs a repaint).
        connect(capture_, &data::Capture::dataAppended, this, [this]{
            updateDataSpan();
            viewport_->update();
            emit dataChanged();
        });
    }
    rebuildTraces();
}

void TraceView::rebuildTraces()
{
    // Reconcile the channel model with the capture. The model preserves
    // any user reordering and (later) keeps derived rows; it emits
    // changed(), which drives the selection clamp, scroll resize and
    // repaint (wired in the constructor). Colours follow the platform
    // theme (dark is primary).
    channels_->syncFromCapture(capture_, util::themeFor(palette()));
    updateDataSpan();
}

data::Signal *TraceView::selectedSignal() const
{
    auto *t = qobject_cast<SignalTrace *>(channels_->at(state_->selectedRow()));
    return t ? t->signal() : nullptr;
}

util::Theme TraceView::theme() const
{
    return util::themeFor(palette());
}

void TraceView::updateDataSpan()
{
    // The rendered time domain is sample 0..sampleCount mapped as
    // 0..(sampleCount/samplerate) seconds (X ignores t0 today).
    double end = 0.0;
    if (capture_) {
        const double sr = capture_->samplerate();
        const qint64 n = capture_->sampleCount();
        if (sr > 0 && n > 0) end = double(n) / sr;
    }
    state_->setDataSpan(0.0, end);
}

void TraceView::syncScrollBars()
{
    // Vertical: pixels of trace stack scrolled off the top.
    const int maxY = std::max(0, viewport_->contentHeight() - viewport_->height());
    // Clamp the current offset into the new range. setYOffset only emits
    // (re-entering here) when the value actually moves, and the clamped
    // value is in range, so this settles after at most one more pass.
    state_->setYOffset(std::max(0, std::min(maxY, state_->yOffset())));
    {
        QSignalBlocker block(vscroll_);
        vscroll_->setRange(0, maxY);
        vscroll_->setPageStep(viewport_->height());
        vscroll_->setSingleStep(state_->rowHeight());
        vscroll_->setValue(state_->yOffset());
        vscroll_->setVisible(maxY > 0);
    }

    // Horizontal: measured in content pixels from the data start. Total
    // content width = span/scale; visible = viewport width.
    const double scale = state_->scale();
    const double span = state_->dataEnd() - state_->dataStart();
    const int visiblePx = viewport_->width();
    const int contentPx = (scale > 0 && span > 0)
                              ? int(span / scale + 0.5) : 0;
    const int maxX = std::max(0, contentPx - visiblePx);
    const int valX = (scale > 0)
        ? int((state_->offset() - state_->dataStart()) / scale + 0.5) : 0;
    {
        QSignalBlocker block(hscroll_);
        hscroll_->setRange(0, maxX);
        hscroll_->setPageStep(visiblePx);
        hscroll_->setSingleStep(std::max(1, visiblePx / 10));
        hscroll_->setValue(std::max(0, std::min(maxX, valX)));
        hscroll_->setVisible(maxX > 0);
    }
}

void TraceView::fitToData()
{
    if (!capture_) return;
    const int w = viewport_->width();
    const double span = state_->dataEnd() - state_->dataStart();
    if (w <= 0 || span <= 0) return;
    // Whole capture across the viewport width, data start at the left.
    state_->setScaleOffset(span / double(w), state_->dataStart());
}

void TraceView::resizeEvent(QResizeEvent *e)
{
    QFrame::resizeEvent(e);
    // The clamp and the fit both depend on the viewport width.
    state_->setViewportWidth(viewport_->width());
    syncScrollBars();
}

void TraceView::zoomIn()  { viewport_->zoom(0.8); }
void TraceView::zoomOut() { viewport_->zoom(1.25); }
void TraceView::toggleCursors() { viewport_->toggleCursors(); }
void TraceView::nextEdge() { viewport_->navigateEdge(+1); }
void TraceView::prevEdge() { viewport_->navigateEdge(-1); }

void TraceView::onDataChanged()
{
    viewport_->update();
}

} // namespace openmso::view
