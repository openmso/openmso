#include "Header.h"

#include "Trace.h"
#include "data/Signal.h"
#include "SignalTrace.h"

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

void Header::paintEvent(QPaintEvent *)
{
    QPainter p(this);
    const QRect r = rect();
    p.fillRect(r, palette().window());

    int y = -st_.yOffset;
    for (const auto &t : traces_) {
        if (!t) continue;
        QRect row(r.left(), y, r.width(), t->height());
        p.setPen(palette().text().color());
        // Color swatch.
        p.fillRect(row.left() + 4, row.top() + 8, 12, 12, t->color());
        p.drawRect(row.left() + 4, row.top() + 8, 12, 12);
        // Name.
        QString name;
        if (auto *st = qobject_cast<SignalTrace *>(t)) {
            if (st->signal()) name = st->signal()->name();
        }
        if (name.isEmpty()) name = QStringLiteral("?");
        p.drawText(row.left() + 22, row.top() + 18, name);
        y += t->height();
    }
}

} // namespace openmso::view
