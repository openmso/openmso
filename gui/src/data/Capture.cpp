#include "Capture.h"

#include "Signal.h"

#include <algorithm>

namespace openmso::data {

Capture::Capture(QObject *parent) : QObject(parent) {}

Signal *Capture::signalById(const QString &id) const
{
    for (auto *s : signals_)
        if (s->id() == id) return s;
    return nullptr;
}

void Capture::beginCapture(double samplerate, double t0,
                           const QList<ChannelSpec> &channels)
{
    setState(State::Arming);
    emit captureBeginning();

    samplerate_ = samplerate;
    t0_ = t0;
    triggerSample_ = -1;
    sampleCount_ = 0;
    errorString_.clear();

    qDeleteAll(signals_);
    signals_.clear();
    for (const auto &c : channels) {
        auto *s = new Signal(c.id, c.name, c.kind, this);
        signals_.append(s);
    }
    setState(State::Capturing);
    emit captureBegan();
}

void Capture::setTriggerSample(qint64 s)
{
    if (triggerSample_ == s) return;
    triggerSample_ = s;
    emit triggerChanged(s);
}

void Capture::markCapturing()
{
    if (state_ != State::Capturing) setState(State::Capturing);
}

void Capture::notifyAppend(qint64 streamId, qint64 firstSample, qint64 nsamples)
{
    sampleCount_ = std::max(sampleCount_, firstSample + nsamples);
    emit dataAppended(streamId, firstSample, nsamples);
}

void Capture::endCapture(bool ok, const QString &errorString)
{
    if (!ok) setErrorString(errorString);
    setState(ok ? State::Complete : State::Error);
    emit captureEnded(ok);
}

void Capture::clear()
{
    qDeleteAll(signals_);
    signals_.clear();
    samplerate_ = 0;
    t0_ = 0;
    triggerSample_ = -1;
    sampleCount_ = 0;
    errorString_.clear();
    setState(State::Idle);
}

void Capture::setState(State s)
{
    if (state_ == s) return;
    state_ = s;
    emit stateChanged(s);
}

void Capture::setErrorString(const QString &s)
{
    if (errorString_ == s) return;
    errorString_ = s;
    emit errorStringChanged(s);
}

} // namespace openmso::data
