#pragma once

#include <QColor>
#include <QObject>
#include <QRect>

#include "view/ViewState.h"

class QPainter;

namespace openmso::data { class Signal; }

namespace openmso::view {

// Base trace. One row in the viewport. Per 06-rendering.md.
// Subclasses: SignalTrace (analog or logic), DecodeTrace (M5).
class Trace : public QObject {
    Q_OBJECT
public:
    Trace(QObject *parent = nullptr);

    int height() const { return height_; }
    void setHeight(int h) { height_ = h; }

    const QColor &color() const { return color_; }
    void setColor(const QColor &c) { color_ = c; }

    // Paint the trace's waveform into `rect` of the viewport. The
    // painter is already translated to `rect.topLeft()`.
    virtual void paintMid(QPainter &p, const QRect &rect,
                          const ViewState &st) = 0;

    // Paint overlays (cursors, hover). Default: nothing.
    virtual void paintFore(QPainter &p, const QRect &rect,
                          const ViewState &st) { Q_UNUSED(p) Q_UNUSED(rect) Q_UNUSED(st) }

signals:
    void heightChanged();

protected:
    int height_ = 80;
    QColor color_;
};

} // namespace openmso::view
