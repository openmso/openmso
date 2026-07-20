#include "SignalTrace.h"

#include "data/Signal.h"

namespace openmso::view {

SignalTrace::SignalTrace(data::Signal *sig, QObject *parent)
    : Trace(parent), sig_(sig)
{
    if (sig_) {
        color_ = sig_->color();
        connect(sig_, &data::Signal::colorChanged,
                this, [this](const QColor &c){ color_ = c; });
    }
}

} // namespace openmso::view
