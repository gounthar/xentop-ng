//! The mem box: host memory and memory by domain.

use super::widgets::*;
use super::{bold, boxed, dim, longest_name, name_width};
use crate::app::App;
use crate::fmt;
use crate::model::{DomRates, Rates};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;
use std::time::Instant;

pub(super) fn mem_box(buf: &mut Buffer, app: &App, r: &Rates, area: Rect) {
    let th = app.theme();
    let h = &r.host;
    let used = h.mem_total.saturating_sub(h.mem_free);
    let uf = used as f64 / h.mem_total.max(1) as f64;
    let block = boxed(
        th,
        "²",
        "mem",
        vec![
            Span::styled(fmt::bytes(h.mem_total as f64), Style::new().fg(th.fg)),
            dim(th, " total"),
        ],
    );
    let inner = block.inner(area);
    block.render(area, buf);
    let w = inner.width as usize;
    if w < 16 || inner.height < 2 {
        return;
    }
    let mut y = inner.y;
    let bottom = inner.y + inner.height;
    let line = |buf: &mut Buffer, y: u16, l: Line| put(buf, inner.x, y, inner.width, &l);

    let mut sp = vec![Span::styled("used ", Style::new().fg(th.fg))];
    sp.extend(meter(uf, w.saturating_sub(5 + 13), &th.mem, th.meter_empty));
    sp.push(bold(format!("{:>7}", fmt::bytes(used as f64)), th.mem.at(uf)));
    sp.push(dim(th, format!("{:>5.0}%", uf * 100.0)));
    line(buf, y, Line::from(sp));
    y += 1;
    if y < bottom {
        line(
            buf,
            y,
            Line::from(vec![
                dim(th, "free "),
                Span::styled(fmt::bytes(h.mem_free as f64), Style::new().fg(th.fg)),
                dim(th, format!("   {} domains", r.domains.len())),
            ]),
        );
        y += 1;
    }
    if y + 1 < bottom {
        line(buf, y, Line::from(dim(th, format!("{:─<w$}", "── by domain "))));
        y += 1;
    }
    let mut doms: Vec<&DomRates> = r.domains.iter().collect();
    doms.sort_by_key(|d| std::cmp::Reverse(d.mem));
    // Sized on the rows drawn (biggest first, a stable order): a wide box
    // (cpu box hidden) shows them in full.
    let room = w.saturating_sub(4 + 1 + 7);
    let shown = doms.iter().take(bottom.saturating_sub(y) as usize).copied();
    let longest = app.frame.name_w.mem.hold(longest_name(shown), Instant::now());
    let nw = name_width(longest, room, (w / 3).clamp(8, 16) - 4) + 4;
    let mw = w.saturating_sub(nw + 1 + 7);
    for d in doms {
        if y >= bottom {
            break;
        }
        let f = d.mem as f64 / h.mem_total.max(1) as f64;
        let mut sp = vec![
            dim(th, format!("{:>3} ", d.id)),
            Span::styled(
                format!("{} ", fmt::pad(&d.name, nw.saturating_sub(4), false)),
                Style::new().fg(th.fg),
            ),
        ];
        // sqrt() so small domains remain visible next to big ones; the
        // value on the right is absolute.
        sp.extend(meter(f.sqrt(), mw, &th.mem, th.meter_empty));
        sp.push(Span::styled(
            format!("{:>7}", fmt::bytes(d.mem as f64)),
            Style::new().fg(th.fg),
        ));
        line(buf, y, Line::from(sp));
        y += 1;
    }
}
