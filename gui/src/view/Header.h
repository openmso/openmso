#pragma once

#include <QList>
#include <QPointer>
#include <QWidget>

#include "view/ViewState.h"

namespace openmso::view {

class Trace;
class ChannelModel;

// Left-side channel labels: name, enable checkbox, color swatch. Rows can
// be drag-reordered, which mutates the shared ChannelModel.
// Per 06-rendering.md.
class Header : public QWidget {
    Q_OBJECT
public:
    explicit Header(QWidget *parent = nullptr);

    // Inject the shared channel model (owned by TraceView). The header
    // reads the row list from it and repaints on changed().
    void setChannelModel(ChannelModel *model);

    // Inject the shared view state (owned by TraceView); repaints on
    // changed() so the labels track the vertical scroll offset.
    void setViewState(ViewState *st);

protected:
    void paintEvent(QPaintEvent *e) override;
    void mousePressEvent(QMouseEvent *e) override;
    void mouseMoveEvent(QMouseEvent *e) override;
    void mouseReleaseEvent(QMouseEvent *e) override;
    QSize sizeHint() const override { return {140, 0}; }

private:
    // Index of the trace row containing viewport-y `y` (accounting for the
    // vertical scroll offset), or -1 if none.
    int rowAt(int y) const;
    // Insertion gap index in [0, count] nearest viewport-y `y` — where a
    // dragged row would drop.
    int insertionGapAt(int y) const;
    // Y of the top of row `row` in widget coords (bottom of the last row
    // when row == count).
    int rowTop(int row) const;
    void refreshFromModel();

    QList<QPointer<Trace>> traces_;   // mirror of model rows, for paint.
    ChannelModel *model_ = nullptr;   // shared, owned by TraceView.
    ViewState *st_ = nullptr;         // shared, owned by TraceView.

    // Drag-to-reorder state. pressRow_ is the row grabbed on press;
    // dragging_ turns on once the pointer moves past a small threshold;
    // dropGap_ is the current insertion gap drawn as a drop indicator.
    int pressRow_ = -1;
    QPoint pressPos_;
    bool dragging_ = false;
    int dropGap_ = -1;
};

} // namespace openmso::view
