#include "ChannelModel.h"

#include "AnalogSignalTrace.h"
#include "LogicSignalTrace.h"
#include "SignalTrace.h"

#include "data/Capture.h"
#include "data/Signal.h"

#include <algorithm>

namespace openmso::view {

ChannelModel::ChannelModel(QObject *parent) : QObject(parent) {}

Trace *ChannelModel::at(int row) const
{
    return (row >= 0 && row < traces_.size()) ? traces_[row] : nullptr;
}

// The data::Signal a trace is backed by, or nullptr for a derived row.
static data::Signal *signalOf(Trace *t)
{
    auto *st = qobject_cast<SignalTrace *>(t);
    return st ? st->signal() : nullptr;
}

// Build (and colour) the trace for a capture signal. One trace per logic
// channel; the channel ordinal is both its bit position in the packed
// segment unit and its resistor-code colour index.
static Trace *makeTrace(data::Signal *sig, util::Theme theme, QObject *owner)
{
    if (sig->kind() == data::SignalKind::Analog) {
        sig->setColor(util::analogColor(sig->channelIndex(), theme));
        auto *t = new AnalogSignalTrace(sig, owner);
        t->setColor(sig->color());
        return t;
    }
    const int bit = std::max(0, sig->channelIndex());
    sig->setColor(util::logicColor(sig->channelIndex(), theme));
    auto *t = new LogicSignalTrace(sig, bit, owner);
    t->setColor(sig->color());
    return t;
}

void ChannelModel::syncFromCapture(data::Capture *cap, util::Theme theme)
{
    const QList<data::Signal *> sigs = cap ? cap->allSignals() : QList<data::Signal *>{};

    // Keep survivors in their current order; drop rows whose capture signal
    // is gone. Derived rows (no backing signal) always survive.
    QList<Trace *> kept;
    QList<Trace *> dropped;
    for (Trace *t : std::as_const(traces_)) {
        if (!t) continue;
        data::Signal *sig = signalOf(t);
        if (!sig || sigs.contains(sig)) {
            // Recolour capture-backed survivors so a theme switch re-tints
            // them (derived rows manage their own colour).
            if (sig) t->setColor(sig->color());
            kept.append(t);
        } else {
            dropped.append(t);
        }
    }

    // Append rows for capture signals not already shown, in capture order.
    for (data::Signal *sig : sigs) {
        if (!sig) continue;
        const bool shown = std::any_of(kept.cbegin(), kept.cend(),
            [sig](Trace *t) { return signalOf(t) == sig; });
        if (!shown)
            kept.append(makeTrace(sig, theme, this));
    }

    for (Trace *t : std::as_const(dropped))
        t->deleteLater();

    traces_ = kept;
    emit changed();
}

void ChannelModel::move(int from, int to)
{
    if (from < 0 || from >= traces_.size()) return;
    if (to < 0 || to >= traces_.size()) return;
    if (from == to) return;
    traces_.move(from, to);
    emit changed();
}

} // namespace openmso::view
