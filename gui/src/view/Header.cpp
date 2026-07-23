#include "Header.h"

#include "Trace.h"
#include "data/Signal.h"
#include "SignalTrace.h"

#include <QFontMetrics>
#include <QMouseEvent>
#include <QPainter>
#include <QPaintEvent>
#include <QPalette>

namespace openmso::view {

Header::Header(QWidget *parent) : QWidget(parent)
{
    setBackgroundRole(QPalette::Window);
    setAutoFillBackground(true);
}

void Header::setTraces(const QList<Trace *> &traces)
{
    traces_.clear();
    for (auto *t : traces)
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

void Header::mousePressEvent(QMouseEvent *e)
{
    if (!st_) return;
    const int row = rowAt(int(e->position().y()));
    if (row >= 0)
        st_->setSelectedRow(row);
}

} // namespace openmso::view
