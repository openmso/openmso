#include "DeriveLogicDialog.h"

#include "data/AnalogSegment.h"
#include "data/Signal.h"
#include "measure/Measure.h"

#include <QCheckBox>
#include <QDialogButtonBox>
#include <QDoubleSpinBox>
#include <QFormLayout>
#include <QLabel>
#include <QVBoxLayout>

namespace openmso::ui {

using namespace openmso::data;

DeriveLogicDialog::DeriveLogicDialog(const Signal *source, double samplerate,
                                     QWidget *parent)
    : QDialog(parent), samplerate_(samplerate)
{
    setWindowTitle(tr("Derive logic channel"));
    setModal(true);

    // Seed the thresholds from the source's real amplitude so the defaults
    // land on a usable pair straddling the midpoint, with a little
    // hysteresis. Fall back to a plain 0 V / symmetric guess if there's no
    // data yet.
    double lo = -1.0, hi = 1.0;
    QString unit = QStringLiteral("V");
    if (auto *seg = qobject_cast<AnalogSegment *>(source->primarySegment())) {
        unit = seg->unit();
        const measure::AnalogStats s =
            measure::measureAnalog(*seg, 0, seg->appendedSamples());
        if (s.valid && s.pp > 0) { lo = s.min; hi = s.max; }
    }
    const double mid = 0.5 * (lo + hi);
    const double pp = hi - lo;
    const double hyst = pp > 0 ? 0.1 * pp : 0.1;   // 10% of Vpp each side.

    auto makeSpin = [&](double val) {
        auto *sp = new QDoubleSpinBox(this);
        sp->setRange(-1e6, 1e6);
        sp->setDecimals(3);
        sp->setSingleStep(pp > 0 ? pp / 50.0 : 0.01);
        sp->setSuffix(QStringLiteral(" ") + unit);
        sp->setValue(val);
        return sp;
    };
    vHigh_ = makeSpin(mid + hyst);   // Vr — rising threshold.
    vLow_ = makeSpin(mid - hyst);    // Vf — falling threshold.

    invert_ = new QCheckBox(tr("Invert output"), this);

    deglitchUs_ = new QDoubleSpinBox(this);
    deglitchUs_->setRange(0.0, 1e6);
    deglitchUs_->setDecimals(3);
    deglitchUs_->setSuffix(tr(" µs"));
    deglitchUs_->setValue(0.0);
    deglitchUs_->setToolTip(tr("Drop pulses shorter than this. 0 disables."));

    auto *form = new QFormLayout;
    form->addRow(tr("Rising threshold (Vr):"), vHigh_);
    form->addRow(tr("Falling threshold (Vf):"), vLow_);
    form->addRow(QString(), invert_);
    form->addRow(tr("De-glitch min width:"), deglitchUs_);

    warning_ = new QLabel(this);
    warning_->setWordWrap(true);
    QPalette wp = warning_->palette();
    wp.setColor(QPalette::WindowText, QColor(220, 150, 60));
    warning_->setPalette(wp);

    auto *buttons = new QDialogButtonBox(
        QDialogButtonBox::Ok | QDialogButtonBox::Cancel, this);
    connect(buttons, &QDialogButtonBox::accepted, this, &QDialog::accept);
    connect(buttons, &QDialogButtonBox::rejected, this, &QDialog::reject);

    auto *lay = new QVBoxLayout(this);
    auto *hint = new QLabel(
        tr("Rising past Vr sets the output high; falling past Vf sets it "
           "low. Keep Vf below Vr for hysteresis (rejects noise at the "
           "crossing)."), this);
    hint->setWordWrap(true);
    lay->addWidget(hint);
    lay->addLayout(form);
    lay->addWidget(warning_);
    lay->addWidget(buttons);

    connect(vHigh_, &QDoubleSpinBox::valueChanged, this,
            &DeriveLogicDialog::validate);
    connect(vLow_, &QDoubleSpinBox::valueChanged, this,
            &DeriveLogicDialog::validate);
    validate();
}

void DeriveLogicDialog::validate()
{
    // Vf < Vr is what gives hysteresis; warn (don't block) if reversed or
    // equal, since equal thresholds are a valid simple comparator.
    if (vLow_->value() > vHigh_->value())
        warning_->setText(tr("⚠ Falling threshold is above rising — this "
                             "inverts the hysteresis and may chatter."));
    else if (qFuzzyCompare(vLow_->value(), vHigh_->value()))
        warning_->setText(tr("⚠ Equal thresholds: no hysteresis, so noise "
                             "at the crossing can produce extra edges."));
    else
        warning_->clear();
}

measure::SchmittParams DeriveLogicDialog::params() const
{
    measure::SchmittParams p;
    p.vHigh = vHigh_->value();
    p.vLow = vLow_->value();
    p.invert = invert_->isChecked();
    p.deglitchSamples = samplerate_ > 0
        ? qint64(deglitchUs_->value() * 1e-6 * samplerate_ + 0.5) : 0;
    return p;
}

} // namespace openmso::ui
