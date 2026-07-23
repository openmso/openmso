#include "MeasurementsPanel.h"

#include "data/AnalogSegment.h"
#include "data/LogicSegment.h"
#include "data/Signal.h"
#include "measure/Measure.h"
#include "util/TimeFormat.h"
#include "view/TraceView.h"
#include "view/ViewState.h"

#include <QHeaderView>
#include <QLabel>
#include <QTableWidget>
#include <QVBoxLayout>

#include <algorithm>
#include <cmath>

namespace openmso::ui {

namespace {

// Format a value with an SI-prefixed unit (e.g. 1.5 kHz, 470 mV).
QString formatSI(double v, const QString &unit)
{
    const double a = std::abs(v);
    if (a == 0.0) return QStringLiteral("0 %1").arg(unit);
    static const struct { double f; const char *p; } pfx[] = {
        {1e9, "G"}, {1e6, "M"}, {1e3, "k"}, {1.0, ""},
        {1e-3, "m"}, {1e-6, "µ"}, {1e-9, "n"},
    };
    for (const auto &e : pfx) {
        if (a >= e.f)
            return QStringLiteral("%1 %2%3")
                .arg(v / e.f, 0, 'g', 4)
                .arg(QString::fromUtf8(e.p), unit);
    }
    return QStringLiteral("%1 n%2").arg(v / 1e-9, 0, 'g', 4).arg(unit);
}

const QString kDash = QStringLiteral("—");   // em dash for N/A.

} // namespace

MeasurementsPanel::MeasurementsPanel(view::TraceView *view, QWidget *parent)
    : QWidget(parent), view_(view)
{
    setMinimumWidth(180);

    auto *layout = new QVBoxLayout(this);
    layout->setContentsMargins(6, 6, 6, 6);
    layout->setSpacing(4);

    auto *title = new QLabel(tr("Measurements"), this);
    QFont tf = title->font();
    tf.setBold(true);
    title->setFont(tf);
    layout->addWidget(title);

    context_ = new QLabel(tr("No channel selected"), this);
    context_->setWordWrap(true);
    layout->addWidget(context_);

    table_ = new QTableWidget(0, 2, this);
    table_->horizontalHeader()->setVisible(false);
    table_->verticalHeader()->setVisible(false);
    table_->setShowGrid(false);
    table_->setSelectionMode(QAbstractItemView::NoSelection);
    table_->setEditTriggers(QAbstractItemView::NoEditTriggers);
    table_->setFocusPolicy(Qt::NoFocus);
    table_->horizontalHeader()->setSectionResizeMode(0, QHeaderView::ResizeToContents);
    table_->horizontalHeader()->setSectionResizeMode(1, QHeaderView::Stretch);
    layout->addWidget(table_, 1);

    if (view_) {
        connect(view_->state(), &view::ViewState::changed,
                this, &MeasurementsPanel::refresh);
        connect(view_, &view::TraceView::dataChanged,
                this, &MeasurementsPanel::refresh);
    }
    refresh();
}

void MeasurementsPanel::refresh()
{
    if (!view_) { setRows(tr("No channel selected"), {}); return; }

    data::Signal *sig = view_->selectedSignal();
    if (!sig || !sig->primarySegment()) {
        setRows(tr("No channel selected"), {});
        return;
    }
    data::Segment *seg = sig->primarySegment();
    const double sr = seg->samplerate();
    view::ViewState *st = view_->state();
    if (sr <= 0.0) {
        setRows(sig->name() + tr(" · no data"), {});
        return;
    }

    // Measurement window: cursor A→B when a real span is selected,
    // otherwise the visible range.
    qint64 first, last;
    QString scope;
    const double a = st->cursorA(), b = st->cursorB();
    if (st->cursorsVisible() && a >= 0 && b >= 0 && a != b) {
        first = qint64(std::min(a, b) * sr);
        last = qint64(std::max(a, b) * sr);
        scope = tr("cursor A–B");
    } else {
        first = qint64(st->xToTime(0) * sr);
        last = qint64(st->xToTime(st->viewportWidth()) * sr);
        scope = tr("visible");
    }
    const QString context =
        QStringLiteral("%1 · %2").arg(sig->name(), scope);

    if (sig->kind() == data::SignalKind::Analog) {
        auto *aseg = qobject_cast<data::AnalogSegment *>(seg);
        if (!aseg) { setRows(context, {}); return; }
        const measure::AnalogStats s = measure::measureAnalog(*aseg, first, last);
        if (!s.valid) { setRows(context, {{tr("(no samples in window)"), kDash}}); return; }
        setRows(context, {
            {tr("Vpp"), formatSI(s.pp, s.unit)},
            {tr("Max"), formatSI(s.max, s.unit)},
            {tr("Min"), formatSI(s.min, s.unit)},
            {tr("Mean"), formatSI(s.mean, s.unit)},
            {tr("RMS"), formatSI(s.rms, s.unit)},
            {tr("Samples"), QString::number(s.sampleCount)},
        });
    } else {
        auto *lseg = qobject_cast<data::LogicSegment *>(seg);
        if (!lseg) { setRows(context, {}); return; }
        const int bit = std::max(0, sig->channelIndex());
        const measure::LogicStats s =
            measure::measureLogic(*lseg, bit, first, last, sr);
        if (!s.valid) { setRows(context, {{tr("(no samples in window)"), kDash}}); return; }
        const QString freq = s.hasTiming ? formatSI(s.frequency, QStringLiteral("Hz")) : kDash;
        const QString period = s.hasTiming ? util::formatTime(s.period) : kDash;
        const QString duty = s.hasTiming
            ? QStringLiteral("%1 %").arg(s.dutyCycle * 100.0, 0, 'f', 1) : kDash;
        const QString wmin = s.posWidthMax > 0 ? util::formatTime(s.posWidthMin) : kDash;
        const QString wmax = s.posWidthMax > 0 ? util::formatTime(s.posWidthMax) : kDash;
        setRows(context, {
            {tr("Frequency"), freq},
            {tr("Period"), period},
            {tr("Duty"), duty},
            {tr("Width (min)"), wmin},
            {tr("Width (max)"), wmax},
            {tr("Edges"), QString::number(s.edgeCount)},
        });
    }
}

void MeasurementsPanel::setRows(const QString &context,
                                const QVector<QPair<QString, QString>> &rows)
{
    context_->setText(context);
    table_->setRowCount(rows.size());
    for (int i = 0; i < rows.size(); ++i) {
        auto *name = new QTableWidgetItem(rows[i].first);
        auto *value = new QTableWidgetItem(rows[i].second);
        value->setTextAlignment(Qt::AlignRight | Qt::AlignVCenter);
        QFont vf = value->font();
        vf.setBold(true);
        value->setFont(vf);
        table_->setItem(i, 0, name);
        table_->setItem(i, 1, value);
    }
}

} // namespace openmso::ui
