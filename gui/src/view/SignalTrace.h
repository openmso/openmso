#pragma once

#include <QString>

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

    // Stable channel id, cached at construction so it survives the source
    // signal being deleted and recreated on re-capture. Reconciliation
    // matches rows by this, never by the (recyclable) Signal pointer.
    const QString &signalId() const { return signalId_; }

    // Point this trace at a new Signal object with the same id (e.g. after
    // a re-capture rebuilt the signal list). Re-subscribes to its color.
    void rebind(data::Signal *sig);

protected:
    data::Signal *sig_;
    QString signalId_;
    QMetaObject::Connection colorConn_;
};

} // namespace openmso::view
