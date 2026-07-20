#pragma once

#include <QObject>

#include "data/LogicSegment.h"
#include "view/SignalTrace.h"

namespace openmso::view {

// Renders a bit-packed logic channel. Per 06-rendering.md.
class LogicSignalTrace : public SignalTrace {
    Q_OBJECT
public:
    LogicSignalTrace(data::Signal *sig, int bitIndex, QObject *parent = nullptr);

    void paintMid(QPainter &p, const QRect &rect,
                  const ViewState &st) override;

private:
    int bitIndex_;
};

} // namespace openmso::view
