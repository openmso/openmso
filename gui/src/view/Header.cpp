#include "Header.h"

#include "ChannelModel.h"
#include "Trace.h"
#include "data/Signal.h"
#include "SignalTrace.h"

#include <QFontMetrics>
#include <QMouseEvent>
#include <QPainter>
#include <QPaintEvent>
#include <QPalette>

#include <cstdlib>

namespace openmso::view {

Header::Header(QWidget *parent) : QWidget(parent)
{
    setBackgroundRole(QPalette::Window);
    setAutoFillBackground(true);
}

void Header::setChannelModel(ChannelModel *model)
{
    if (model_ == model) return;
    if (model_) disconnect(model_, nullptr, this, nullptr);
    model_ = model;
    if (model_)
        connect(model_, &ChannelModel::changed, this,
                &Header::refreshFromModel);
    refreshFromModel();
}

void Header::refreshFromModel()
{
    traces_.clear();
    if (model_)
        for (auto *t : model_->traces())
            if (t) traces_.append(t);
    update();
}

void Header::setViewState(ViewState *st)
{
    if (st_ == st) return;
    if (st_) disconnect(st_, nullptr, this, nullptr);
    st_ = st;
    if (st_)
        connect(st_, &ViewState::changed, this,
                qOverload<>(&QWidget::update));
    update();
}

void Header::paintEvent(QPaintEvent *)
{
    QPainter p(this);
    const QRect r = rect();
    p.fillRect(r, palette().window());

    const QFontMetrics fm = p.fontMetrics();
    const int selected = st_ ? st_->selectedRow() : -1;
    int y = st_ ? -st_->yOffset() : 0;
    int rowIndex = 0;
    for (const auto &t : traces_) {
        if (!t) { ++rowIndex; continue; }
        QRect row(r.left(), y, r.width(), t->height());
        // Highlight the selected row so it's clear which channel the
        // cursor snap and n/N edge navigation act on.
        if (rowIndex == selected) {
            QColor hl = palette().highlight().color();
            hl.setAlpha(48);
            p.fillRect(row, hl);
        }
        // Vertically centre the swatch and label on the waveform's
        // midline (top + h/2), where the logic high/low pair and the
        // analog baseline are centred, so the label lines up with its
        // channel instead of floating at the row top.
        const int mid = row.top() + row.height() / 2;
        p.setPen(palette().text().color());
        // Color swatch.
        p.fillRect(row.left() + 4, mid - 6, 12, 12, t->color());
        p.drawRect(row.left() + 4, mid - 6, 12, 12);
        // Name, baseline centred on the midline.
        QString name;
        if (auto *st = qobject_cast<SignalTrace *>(t)) {
            if (st->signal()) name = st->signal()->name();
        }
        if (name.isEmpty()) name = QStringLiteral("?");
        if (rowIndex == selected) {
            QFont f = p.font();
            f.setBold(true);
            p.setFont(f);
            p.drawText(row.left() + 22, mid + fm.ascent() / 2 - 1, name);
            f.setBold(false);
            p.setFont(f);
        } else {
            p.drawText(row.left() + 22, mid + fm.ascent() / 2 - 1, name);
        }
        y += t->height();
        ++rowIndex;
    }

    // Drop indicator: a bright line at the gap where a dragged row lands.
    if (dragging_ && dropGap_ >= 0) {
        const int yLine = rowTop(dropGap_);
        p.setPen(QPen(palette().highlight().color(), 2));
        p.drawLine(r.left() + 2, yLine, r.right() - 2, yLine);
    }
}

int Header::rowAt(int y) const
{
    int top = st_ ? -st_->yOffset() : 0;
    int i = 0;
    for (const auto &t : traces_) {
        if (!t) { ++i; continue; }
        const int h = t->height();
        if (y >= top && y < top + h) return i;
        top += h;
        ++i;
    }
    return -1;
}

int Header::insertionGapAt(int y) const
{
    int top = st_ ? -st_->yOffset() : 0;
    int i = 0;
    for (const auto &t : traces_) {
        if (!t) { ++i; continue; }
        const int h = t->height();
        if (y < top + h / 2) return i;
        top += h;
        ++i;
    }
    return traces_.size();
}

int Header::rowTop(int row) const
{
    int top = st_ ? -st_->yOffset() : 0;
    for (int i = 0; i < row && i < traces_.size(); ++i)
        if (traces_[i]) top += traces_[i]->height();
    return top;
}

// New selected-row index after moving the row at `from` to `to`.
static int adjustForMove(int sel, int from, int to)
{
    if (sel < 0) return sel;
    if (sel == from) return to;
    if (from < sel && sel <= to) return sel - 1;
    if (to <= sel && sel < from) return sel + 1;
    return sel;
}

void Header::mousePressEvent(QMouseEvent *e)
{
    if (!st_ || e->button() != Qt::LeftButton) return;
    pressPos_ = e->position().toPoint();
    pressRow_ = rowAt(pressPos_.y());
    dragging_ = false;
    dropGap_ = -1;
    if (pressRow_ >= 0)
        st_->setSelectedRow(pressRow_);
}

void Header::mouseMoveEvent(QMouseEvent *e)
{
    if (pressRow_ < 0 || !(e->buttons() & Qt::LeftButton)) return;
    const QPoint pos = e->position().toPoint();
    // Small dead zone so a click doesn't register as a drag.
    if (!dragging_ && std::abs(pos.y() - pressPos_.y()) < 4) return;
    dragging_ = true;
    dropGap_ = insertionGapAt(pos.y());
    update();
}

void Header::mouseReleaseEvent(QMouseEvent *)
{
    if (dragging_ && model_ && pressRow_ >= 0 && dropGap_ >= 0) {
        // Translate the insertion gap (0..count) to a QList::move target.
        const int to = dropGap_ > pressRow_ ? dropGap_ - 1 : dropGap_;
        if (to != pressRow_) {
            model_->move(pressRow_, to);
            if (st_)
                st_->setSelectedRow(adjustForMove(st_->selectedRow(),
                                                  pressRow_, to));
        }
    }
    pressRow_ = -1;
    dragging_ = false;
    dropGap_ = -1;
    update();
}

} // namespace openmso::view
