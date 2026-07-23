#include "ChannelModel.h"

#include "AnalogSignalTrace.h"
#include "DerivedChannel.h"
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

// Apply the curated palette colour for `sig` to both the signal and its
// trace. One trace per logic channel; the channel ordinal is both its bit
// position in the packed unit and its resistor-code colour index.
static void applyColor(data::Signal *sig, Trace *t, util::Theme theme)
{
    const QColor c = sig->kind() == data::SignalKind::Analog
        ? util::analogColor(sig->channelIndex(), theme)
        : util::logicColor(sig->channelIndex(), theme);
    sig->setColor(c);
    t->setColor(c);
}

static Trace *makeTrace(data::Signal *sig, util::Theme theme, QObject *owner)
{
    Trace *t = sig->kind() == data::SignalKind::Analog
        ? static_cast<Trace *>(new AnalogSignalTrace(sig, owner))
        : static_cast<Trace *>(
              new LogicSignalTrace(sig, std::max(0, sig->channelIndex()), owner));
    applyColor(sig, t, theme);
    return t;
}

void ChannelModel::syncFromCapture(data::Capture *cap, util::Theme theme)
{
    const QList<data::Signal *> sigs = cap ? cap->allSignals()
                                           : QList<data::Signal *>{};

    // IMPORTANT: reconcile by stable channel *id*, never by Signal pointer.
    // A re-capture deletes every old Signal and allocates new ones, and the
    // allocator readily recycles the freed addresses — so a trace's old
    // Signal* can coincidentally equal a *different* new Signal, which would
    // bind rows to the wrong channel in a nondeterministic order. The id
    // (cached on the trace) survives the delete; the pointer does not.
    QHash<QString, data::Signal *> byId;
    byId.reserve(sigs.size());
    for (data::Signal *sig : sigs)
        if (sig) byId.insert(sig->id(), sig);

    // Keep survivors in their current (possibly user-reordered) order;
    // rebind each kept capture row to its new Signal object. Derived rows
    // always survive — their synthetic signal isn't rebuilt by the capture.
    QList<Trace *> kept;
    QList<Trace *> dropped;
    QSet<QString> shownIds;
    for (Trace *t : std::as_const(traces_)) {
        if (!t) continue;
        if (derivedTraces_.contains(t)) {
            kept.append(t);
            continue;
        }
        auto *st = qobject_cast<SignalTrace *>(t);
        data::Signal *sig = st ? byId.value(st->signalId(), nullptr) : nullptr;
        if (sig) {
            st->rebind(sig);            // point at the fresh Signal object.
            applyColor(sig, t, theme);
            shownIds.insert(st->signalId());
            kept.append(t);
        } else {
            dropped.append(t);
        }
    }

    // Append rows for capture signals not already shown, in capture order.
    for (data::Signal *sig : sigs) {
        if (!sig || shownIds.contains(sig->id())) continue;
        kept.append(makeTrace(sig, theme, this));
    }

    for (Trace *t : std::as_const(dropped)) {
        derivedTraces_.remove(t);
        t->deleteLater();
    }

    traces_ = kept;

    // Rebind any derived channels' source pointers to the new Signal
    // objects (same id), so they keep recomputing after a re-capture
    // instead of dangling on a deleted source.
    for (DerivedChannel *dc : std::as_const(derived_)) {
        data::Signal *src = byId.value(dc->sourceId(), nullptr);
        if (src) dc->rebindSource(src);
    }

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

DerivedChannel *ChannelModel::addDerived(data::Signal *source,
                                         const measure::SchmittParams &params,
                                         util::Theme theme)
{
    if (!source || source->kind() != data::SignalKind::Analog) return nullptr;

    auto *dc = new DerivedChannel(source, params, this);
    derived_.append(dc);
    derivedSignals_.insert(dc->signal());

    // A single-bit logic lane over the synthetic signal. Give it the
    // source's analog colour so the relationship reads at a glance.
    auto *t = new LogicSignalTrace(dc->signal(), 0, this);
    t->setColor(util::analogColor(source->channelIndex(), theme));
    derivedTraces_.insert(t);

    // Insert directly below the source row if it's shown, else append.
    int at = traces_.size();
    for (int i = 0; i < traces_.size(); ++i) {
        auto *st = qobject_cast<SignalTrace *>(traces_[i]);
        if (st && st->signalId() == source->id()) { at = i + 1; break; }
    }
    traces_.insert(at, t);

    // A finished walk swaps in fresh bits (emitting on the synthetic
    // signal, which repaints the trace); surface it as a model change too so
    // the scroll range / any dependent UI refresh.
    connect(dc, &DerivedChannel::computed, this, &ChannelModel::changed);

    emit changed();
    return dc;
}

DerivedChannel *ChannelModel::derivedFor(Trace *t) const
{
    if (!derivedTraces_.contains(t)) return nullptr;
    auto *st = qobject_cast<SignalTrace *>(t);
    if (!st) return nullptr;
    for (DerivedChannel *dc : derived_)
        if (dc->signal() == st->signal()) return dc;
    return nullptr;
}

} // namespace openmso::view
