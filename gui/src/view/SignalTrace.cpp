#include "SignalTrace.h"

#include "data/Signal.h"

namespace openmso::view {

SignalTrace::SignalTrace(data::Signal *sig, QObject *parent)
    : Trace(parent), sig_(sig)
{
    if (sig_) {
        signalId_ = sig_->id();
        color_ = sig_->color();
        colorConn_ = connect(sig_, &data::Signal::colorChanged,
                             this, [this](const QColor &c){ color_ = c; });
    }
}

void SignalTrace::rebind(data::Signal *sig)
{
    if (sig_ == sig) return;
    // Sever the old color subscription via its handle. Disconnecting a
    // Connection is safe even when the old signal has already been
    // destroyed (the recapture case) — it never dereferences the sender.
    QObject::disconnect(colorConn_);
    sig_ = sig;
    if (sig_) {
        signalId_ = sig_->id();
        color_ = sig_->color();
        colorConn_ = connect(sig_, &data::Signal::colorChanged,
                             this, [this](const QColor &c){ color_ = c; });
    }
}

} // namespace openmso::view
