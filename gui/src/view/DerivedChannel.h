#pragma once

#include <QByteArray>
#include <QFutureWatcher>
#include <QObject>
#include <QString>
#include <QTimer>

#include "measure/Schmitt.h"

namespace openmso::data { class Signal; }

namespace openmso::view {

// A view-layer channel derived from an analog source via a dual-threshold
// Schmitt trigger (measure::schmittWalk). It owns a synthetic logic
// data::Signal that renders as an ordinary logic lane — so it is
// automatically cursor/snap/edge-nav/measurement capable and can later feed
// a decoder. Per the locked decision (docs/gui-plan HANDOFF): derived
// channels live in the view layer, not the Capture.
//
// The walk runs on a worker thread (QtConcurrent) so a large capture never
// blocks the GUI. Recompute is debounced (coalesces slider spam and live
// append bursts) and coalesced (a change during a run re-runs once it
// finishes). The snapshot of the source samples is taken on the GUI thread
// under the segment read lock, so the worker touches no live segment.
class DerivedChannel : public QObject {
    Q_OBJECT
public:
    DerivedChannel(data::Signal *source, const measure::SchmittParams &params,
                   QObject *parent = nullptr);

    data::Signal *source() const { return source_; }
    data::Signal *signal() const { return out_; }   // synthetic logic signal
    measure::SchmittParams params() const { return params_; }
    void setParams(const measure::SchmittParams &p);

    // Stable id of the source channel, cached so it survives the source
    // being deleted/recreated on re-capture (see ChannelModel).
    const QString &sourceId() const { return sourceId_; }
    // Point at the fresh source Signal (same id) after a re-capture and
    // recompute. Re-subscribes to the new signal's data changes.
    void rebindSource(data::Signal *source);

signals:
    void computed();   // fresh bits swapped into signal()

private:
    void connectSource();
    void scheduleRecompute();
    void launch();
    void onFinished();

    data::Signal *source_;
    QString sourceId_;                  // stable, survives source recreation.
    data::Signal *out_;                 // child of this.
    measure::SchmittParams params_;
    double samplerate_ = 0.0;

    QTimer debounce_;
    QFutureWatcher<QByteArray> watcher_;
    bool dirty_ = false;   // a change arrived while a walk was in flight.
    QList<QMetaObject::Connection> sourceConns_;
};

} // namespace openmso::view
