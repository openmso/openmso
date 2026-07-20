#include "Signal.h"

#include "Segment.h"

namespace openmso::data {

Signal::Signal(QString id, QString name, SignalKind kind, QObject *parent)
    : QObject(parent), id_(std::move(id)), name_(std::move(name)),
      kind_(kind)
{
    // Default color: a placeholder; the view layer assigns curated
    // palette colors at layout time.
    color_ = QColor(Qt::darkCyan);
}

void Signal::setEnabled(bool e)
{
    if (enabled_ == e) return;
    enabled_ = e;
    emit enabledChanged(e);
}

void Signal::setColor(const QColor &c)
{
    if (color_ == c) return;
    color_ = c;
    emit colorChanged(c);
}

void Signal::appendSegment(Segment *s)
{
    s->setParent(this);
    segments_.append(s);
    emit dataChanged();
}

void Signal::clearSegments()
{
    qDeleteAll(segments_);
    segments_.clear();
    emit segmentsReset();
}

} // namespace openmso::data
