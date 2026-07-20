#pragma once

#include <QWidget>

#include "view/ViewState.h"

namespace openmso::view {

// Time axis with ticks and cursor labels. Per 06-rendering.md.
class Ruler : public QWidget {
    Q_OBJECT
public:
    explicit Ruler(QWidget *parent = nullptr);

    void setState(const ViewState &st) { st_ = st; update(); }

protected:
    void paintEvent(QPaintEvent *e) override;
    QSize sizeHint() const override { return {0, 28}; }

private:
    ViewState st_;
};

} // namespace openmso::view
