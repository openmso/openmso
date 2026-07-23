#pragma once

#include <QDialog>

#include "measure/Schmitt.h"

class QCheckBox;
class QDoubleSpinBox;
class QLabel;
class QSpinBox;

namespace openmso::data { class Signal; }

namespace openmso::ui {

// "Derive logic channel…" — configure a dual-threshold Schmitt trigger
// (Vr rising, Vf falling) plus invert and de-glitch, for turning an analog
// channel into a logic lane. Modal; returns the chosen params on accept.
//
// The dialog only gathers parameters — the actual (potentially expensive)
// walk runs off-thread inside view::DerivedChannel once the row is added,
// so there's no heavy work on the GUI thread here.
class DeriveLogicDialog : public QDialog {
    Q_OBJECT
public:
    DeriveLogicDialog(const data::Signal *source, double samplerate,
                      QWidget *parent = nullptr);

    measure::SchmittParams params() const;

private:
    void validate();

    double samplerate_;
    QDoubleSpinBox *vHigh_;
    QDoubleSpinBox *vLow_;
    QCheckBox *invert_;
    QDoubleSpinBox *deglitchUs_;   // minimum pulse width, microseconds.
    QLabel *warning_;
};

} // namespace openmso::ui
