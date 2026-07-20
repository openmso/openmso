#pragma once

#include <QColor>
#include <QObject>
#include <QString>

#include "data/Types.h"

namespace openmso::data {

class Segment;

// One channel (analog or logic) in one capture. Owns its segments
// (usually one; >1 for segmented acquisitions). Per
// docs/gui-plan/05-data-model.md.
class Signal : public QObject {
    Q_OBJECT
public:
    Signal(QString id, QString name, SignalKind kind,
           QObject *parent = nullptr);

    const QString &id() const { return id_; }
    const QString &name() const { return name_; }
    SignalKind kind() const { return kind_; }

    bool enabled() const { return enabled_; }
    void setEnabled(bool e);

    QColor color() const { return color_; }
    void setColor(const QColor &c);

    // Segment management. The signal takes ownership.
    QList<Segment *> segments() const { return segments_; }
    Segment *primarySegment() const {
        return segments_.isEmpty() ? nullptr : segments_.first();
    }
    void appendSegment(Segment *s);
    void clearSegments();

signals:
    void enabledChanged(bool);
    void colorChanged(const QColor &);
    void dataChanged();          // segment appended or chunk appended
    void segmentsReset();

private:
    QString id_;
    QString name_;
    SignalKind kind_;
    bool enabled_ = true;
    QColor color_;
    QList<Segment *> segments_;  // owned
};

} // namespace openmso::data
