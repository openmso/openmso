#pragma once

#include "Trace.h"

namespace openmso::data { class Signal; }

namespace openmso::view {

// Base for traces backed by a data::Signal. Holds a non-owning pointer
// to the signal; the data layer owns it.
class SignalTrace : public Trace {
    Q_OBJECT
public:
    SignalTrace(data::Signal *sig, QObject *parent = nullptr);

    data::Signal *signal() const { return sig_; }

protected:
    data::Signal *sig_;
};

} // namespace openmso::view
