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
    void setState(const ViewState &st) { st_ = st; update(); }

protected:
    void paintEvent(QPaintEvent *e) override;
    QSize sizeHint() const override { return {140, 0}; }

private:
    QList<QPointer<Trace>> traces_;
    ViewState st_;
};

} // namespace openmso::view
