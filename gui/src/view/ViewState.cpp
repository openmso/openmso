#include "ViewState.h"

#include <algorithm>

namespace openmso::view {

double ViewState::clampScale(double s)
{
    return std::max(1e-12, std::min(1e3, s));
}

double ViewState::fitScale() const
{
    if (viewportWidth_ <= 0 || dataEnd_ <= dataStart_) return 0.0;
    return (dataEnd_ - dataStart_) / double(viewportWidth_);
}

void ViewState::clampView()
{
    // No extent or no width yet ⇒ the view is unconstrained.
    if (viewportWidth_ <= 0 || dataEnd_ <= dataStart_) return;

    const double fit = (dataEnd_ - dataStart_) / double(viewportWidth_);
    // Never zoom out past a whole-capture fit (that would show margins).
    if (scale_ > fit) scale_ = fit;

    // Keep [offset, offset + visible] inside [dataStart, dataEnd].
    const double visible = viewportWidth_ * scale_;
    double maxOffset = dataEnd_ - visible;
    if (maxOffset < dataStart_) maxOffset = dataStart_;  // fit ⇒ pinned.
    offset_ = std::max(dataStart_, std::min(maxOffset, offset_));
}

void ViewState::setScale(double s)
{
    const double os = scale_, oo = offset_;
    scale_ = clampScale(s);
    clampView();
    if (scale_ != os || offset_ != oo) emit changed();
}

void ViewState::setOffset(double o)
{
    const double os = scale_, oo = offset_;
    offset_ = o;
    clampView();
    if (scale_ != os || offset_ != oo) emit changed();
}

void ViewState::setScaleOffset(double s, double o)
{
    const double os = scale_, oo = offset_;
    scale_ = clampScale(s);
    offset_ = o;
    clampView();
    if (scale_ != os || offset_ != oo) emit changed();
}

void ViewState::setDataSpan(double start, double end)
{
    if (start == dataStart_ && end == dataEnd_) return;
    dataStart_ = start;
    dataEnd_ = end;
    const double os = scale_, oo = offset_;
    clampView();
    if (scale_ != os || offset_ != oo) emit changed();
}

void ViewState::setViewportWidth(int w)
{
    if (w == viewportWidth_) return;
    viewportWidth_ = w;
    const double os = scale_, oo = offset_;
    clampView();
    if (scale_ != os || offset_ != oo) emit changed();
}

void ViewState::setRowHeight(int h)
{
    if (h == rowHeight_) return;
    rowHeight_ = h;
    emit changed();
}

void ViewState::setYOffset(int y)
{
    if (y == yOffset_) return;
    yOffset_ = y;
    emit changed();
}

void ViewState::setCursors(double a, double b)
{
    if (a == cursorA_ && b == cursorB_) return;
    cursorA_ = a;
    cursorB_ = b;
    emit cursorMoved(cursorA_, cursorB_);
    emit changed();
}

void ViewState::setCursorsVisible(bool v)
{
    if (v == cursorsVisible_) return;
    cursorsVisible_ = v;
    emit changed();
}

void ViewState::setTriggerPos(double t)
{
    // NaN != NaN, so guard the all-NaN no-op explicitly.
    if (t == triggerPos_ || (qIsNaN(t) && qIsNaN(triggerPos_))) return;
    triggerPos_ = t;
    emit changed();
}

} // namespace openmso::view
