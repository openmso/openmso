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

    // Bit position of this channel within the packed logic unit — the
    // key for edge-index queries (snap, next/prev-edge navigation).
    int bitIndex() const { return bitIndex_; }

private:
    int bitIndex_;
};

} // namespace openmso::view
