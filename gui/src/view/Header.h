#pragma once

#include <QList>
#include <QPointer>
#include <QWidget>

#include "view/ViewState.h"

namespace openmso::view {

class Trace;

// Left-side channel labels: name, enable checkbox, color swatch.
// Per 06-rendering.md.
class Header : public QWidget {
    Q_OBJECT
public:
    explicit Header(QWidget *parent = nullptr);

    void setTraces(const QList<Trace *> &traces);

    // Inject the shared view state (owned by TraceView); repaints on
    // changed() so the labels track the vertical scroll offset.
    void setViewState(ViewState *st);

protected:
    void paintEvent(QPaintEvent *e) override;
    QSize sizeHint() const override { return {140, 0}; }

private:
    QList<QPointer<Trace>> traces_;
    ViewState *st_ = nullptr;   // shared, owned by TraceView.
};

} // namespace openmso::view
