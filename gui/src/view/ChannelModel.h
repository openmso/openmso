#pragma once

#include <QList>
#include <QObject>
#include <QSet>

#include "measure/Schmitt.h"
#include "util/ChannelColors.h"

namespace openmso::data { class Capture; class Signal; }

namespace openmso::view {

class Trace;
class DerivedChannel;

// The ordered, mutable list of channel rows shown in the view. It is the
// single source of truth for row order and membership — decoupled from
// data::Capture, which stays "what the device sent". Today every row is
// backed by a capture signal; the model exists so rows can be reordered
// (header drag) and, next, so it can hold *derived* channels that have no
// capture backing (analog->logic, buses). Per docs/gui-plan HANDOFF.
//
// Owns its Trace objects (parented to this). The Header and Viewport hold
// a pointer to the model and repaint on changed(); TraceView keeps it in
// sync with the capture.
class ChannelModel : public QObject {
    Q_OBJECT
public:
    explicit ChannelModel(QObject *parent = nullptr);

    // Reconcile the row list with a capture: keep existing rows whose
    // backing signal still exists (preserving the current, possibly
    // user-reordered, order), append rows for signals not yet shown, and
    // drop rows whose signal disappeared. Derived rows (no capture signal)
    // are always kept. Idempotent; safe to call on every channelsChanged.
    void syncFromCapture(data::Capture *cap, util::Theme theme);

    const QList<Trace *> &traces() const { return traces_; }
    int count() const { return traces_.size(); }
    Trace *at(int row) const;
    int indexOf(Trace *t) const { return traces_.indexOf(t); }

    // Move the row at `from` to index `to` (QList::move semantics). No-op
    // if either index is out of range or from == to. Emits changed().
    void move(int from, int to);

    // Add a derived logic channel: a dual-threshold Schmitt trigger over an
    // analog `source` signal. Creates a DerivedChannel (owns the async walk
    // and the synthetic signal) and a logic row for it, inserted directly
    // below the source. The row survives capture re-syncs and reorders.
    // Returns the DerivedChannel (owned by the model), or nullptr if the
    // source isn't usable. Emits changed().
    DerivedChannel *addDerived(data::Signal *source,
                               const measure::SchmittParams &params,
                               util::Theme theme);

    // The DerivedChannel whose output backs `t`, or nullptr if `t` is not a
    // derived row. Lets the UI reopen the derive dialog to re-tune a row.
    DerivedChannel *derivedFor(Trace *t) const;

signals:
    void changed();   // membership or order changed — repaint / rescroll

private:
    QList<Trace *> traces_;              // owned (parented to this)
    QList<DerivedChannel *> derived_;    // owned (parented to this)
    QSet<data::Signal *> derivedSignals_;  // derived output signals
    QSet<Trace *> derivedTraces_;          // rows to keep across re-sync
};

} // namespace openmso::view
