#include "DerivedChannel.h"

#include "data/AnalogSegment.h"
#include "data/LogicSegment.h"
#include "data/Signal.h"

#include <QReadLocker>
#include <QtConcurrent>

namespace openmso::view {

using namespace openmso::data;

DerivedChannel::DerivedChannel(Signal *source,
                               const measure::SchmittParams &params,
                               QObject *parent)
    : QObject(parent), source_(source), sourceId_(source->id()), params_(params)
{
    // Synthetic logic signal. A distinct id/name so it reads as derived and
    // never collides with a captured channel.
    const QString id = source_->id() + QStringLiteral(":logic");
    const QString name = source_->name() + QStringLiteral(" ⟂");
    out_ = new Signal(id, name, SignalKind::Logic, this);

    debounce_.setSingleShot(true);
    debounce_.setInterval(40);   // ms — coalesce bursts of changes.
    connect(&debounce_, &QTimer::timeout, this, &DerivedChannel::launch);
    connect(&watcher_, &QFutureWatcher<QByteArray>::finished,
            this, &DerivedChannel::onFinished);

    connectSource();
    scheduleRecompute();
}

void DerivedChannel::connectSource()
{
    // Recompute whenever the source data grows (live capture) or resets.
    sourceConns_.append(connect(source_, &Signal::dataChanged, this,
                                &DerivedChannel::scheduleRecompute));
    sourceConns_.append(connect(source_, &Signal::segmentsReset, this,
                                &DerivedChannel::scheduleRecompute));
}

void DerivedChannel::rebindSource(Signal *source)
{
    if (!source || source == source_) return;
    // The old source may already be destroyed (re-capture) — disconnect via
    // the stored handles, which never dereference the old sender.
    for (const auto &c : std::as_const(sourceConns_))
        QObject::disconnect(c);
    sourceConns_.clear();
    source_ = source;
    sourceId_ = source_->id();
    connectSource();
    scheduleRecompute();
}

void DerivedChannel::setParams(const measure::SchmittParams &p)
{
    params_ = p;
    scheduleRecompute();
}

void DerivedChannel::scheduleRecompute()
{
    // A change during an in-flight walk is remembered and re-run on finish,
    // rather than launching an overlapping second walk.
    if (watcher_.isRunning()) {
        dirty_ = true;
        return;
    }
    debounce_.start();
}

void DerivedChannel::launch()
{
    auto *seg = qobject_cast<AnalogSegment *>(source_->primarySegment());
    if (!seg) return;

    // Snapshot on the GUI thread under the read lock: copy-on-write makes
    // the byte grab O(1), and the decode params come with it. The worker
    // then walks this snapshot with no lock and no live-segment reference —
    // so a concurrent append can't race it, and a later delete of the
    // source can't dangle under it.
    QByteArray raw;
    AnalogDType dt;
    double scale, offset;
    qint64 n;
    {
        QReadLocker l(&seg->lock);
        raw = seg->rawBytes();
        dt = seg->dtype();
        scale = seg->scale();
        offset = seg->offset();
        n = seg->appendedSamples();
    }
    samplerate_ = seg->samplerate();
    dirty_ = false;

    watcher_.setFuture(QtConcurrent::run(measure::schmittWalk, raw, dt,
                                         scale, offset, n, params_));
}

void DerivedChannel::onFinished()
{
    const QByteArray bits = watcher_.result();

    // Swap the freshly computed bits into the synthetic signal on the GUI
    // thread. clearSegments()/appendSegment() emit on Signal, which drives
    // the trace repaint and a lazy edge-index rebuild.
    auto *seg = new LogicSegment(1, 1);
    seg->setSamplerate(samplerate_);
    seg->appendChunk(bits, 0, bits.size());
    out_->clearSegments();
    out_->appendSegment(seg);

    emit computed();

    // A change arrived mid-walk — run once more with the latest input.
    if (dirty_)
        scheduleRecompute();
}

} // namespace openmso::view
