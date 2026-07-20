#pragma once

#include <QList>
#include <QObject>
#include <QString>

#include "data/Signal.h"
#include "data/Types.h"

namespace openmso::data {

class Signal;

// One acquisition. Owns its signals. State machine: Idle → Arming →
// Capturing → Complete | Error. Per 05-data-model.md.
class Capture : public QObject {
    Q_OBJECT
public:
    enum class State {
        Idle,
        Arming,
        Capturing,
        Complete,
        Error,
    };
    Q_ENUM(State)

    explicit Capture(QObject *parent = nullptr);

    State state() const { return state_; }
    double samplerate() const { return samplerate_; }
    double t0() const { return t0_; }
    qint64 triggerSample() const { return triggerSample_; }
    qint64 sampleCount() const { return sampleCount_; }
    const QString &errorString() const { return errorString_; }

    QList<Signal *> allSignals() const { return signals_; }
    Signal *signalById(const QString &id) const;

    // --- mutation API (called on GUI thread by the Session controller) ---

    // Begin a new acquisition. Clears any prior data, creates Signal
    // objects from the describe() channel list, sets samplerate/t0.
    // `channelSpec` is a list of {id, kind, name} tuples in display
    // order.
    struct ChannelSpec {
        QString id;
        QString name;
        SignalKind kind;
    };
    void beginCapture(double samplerate, double t0,
                      const QList<ChannelSpec> &channels);

    // Set the trigger position (sample index, -1 = unknown).
    void setTriggerSample(qint64 s);

    // Mark the capture as live (data arriving).
    void markCapturing();

    // Record that `n` samples were appended to `streamId` (for the
    // status bar sample-count readout).
    void notifyAppend(qint64 streamId, qint64 firstSample, qint64 nsamples);

    // End the acquisition. `ok=true` → Complete; `ok=false` → Error
    // with `errorString`.
    void endCapture(bool ok, const QString &errorString = {});

    // Clear everything (resets to Idle with no signals).
    void clear();

signals:
    void stateChanged(State s);
    void captureBeginning();    // before signals are created
    void captureBegan();
    void triggerChanged(qint64 sample);
    void dataAppended(qint64 streamId, qint64 firstSample, qint64 nsamples);
    void captureEnded(bool ok);
    void errorStringChanged(const QString &);

private:
    void setState(State s);
    void setErrorString(const QString &s);

    State state_ = State::Idle;
    double samplerate_ = 0.0;
    double t0_ = 0.0;
    qint64 triggerSample_ = -1;
    qint64 sampleCount_ = 0;
    QString errorString_;
    QList<Signal *> signals_;  // owned
};

} // namespace openmso::data
