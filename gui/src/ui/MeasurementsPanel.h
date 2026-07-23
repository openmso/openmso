#pragma once

#include <QWidget>

class QLabel;
class QTableWidget;

namespace openmso::view { class TraceView; }

namespace openmso::ui {

// Panel of automatic measurements for the selected channel, computed over
// the cursor A→B window when cursors are down, otherwise over the visible
// range. Recomputes live on view/cursor/selection moves and on new data.
//
// Deliberately a plain QWidget hosted in a QSplitter (not a QDockWidget):
// dock separator resize relies on QWidget::grabMouse(), which native
// Wayland refuses for non-popup surfaces. A splitter handle resizes via
// Qt's implicit press-grab, which Wayland supports — so this stays crisp
// under fractional scaling. Per docs/gui-plan/11-milestones.md.
class MeasurementsPanel : public QWidget {
    Q_OBJECT
public:
    explicit MeasurementsPanel(view::TraceView *view, QWidget *parent = nullptr);

private:
    void refresh();
    void setRows(const QString &context,
                 const QVector<QPair<QString, QString>> &rows);

    view::TraceView *view_;
    QLabel *context_;
    QTableWidget *table_;
};

} // namespace openmso::ui
