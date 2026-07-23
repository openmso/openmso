#pragma once

#include <QList>
#include <QObject>

#include "util/ChannelColors.h"

namespace openmso::data { class Capture; }

namespace openmso::view {

class Trace;

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

signals:
    void changed();   // membership or order changed — repaint / rescroll

private:
    QList<Trace *> traces_;   // owned (parented to this)
};

} // namespace openmso::view
