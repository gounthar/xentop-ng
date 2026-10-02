//! The cpu box: host load graph and per-pCPU views (list, heatmap), or
//! the hungriest domains when there is no per-pCPU data.

use super::widgets::*;
use super::{bold, boxed, dim, longest_name, name_width, steal_color};
use crate::app::App;
use crate::fmt;
use crate::model::{DomRates, Rates};
use crate::theme::Theme;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;
use std::time::Instant;

pub(super) fn cpu_box(buf: &mut Buffer, app: &App, r: &Rates, area: Rect) {
    let th = app.theme();
    let h = &r.host;
    let right = vec![
        Span::styled(format!("{}", h.num_cpus), Style::new().fg(th.fg)),
        dim(th, " pCPU"),
        if h.cpu_mhz > 0 {
            dim(th, format!(" @ {:.2} GHz", h.cpu_mhz as f64 / 1000.0))
        } else {
            Span::raw("")
        },
    ];
    let block = boxed(th, "¹", "cpu", right);
    let inner = block.inner(area);
    block.render(area, buf);
    if inner.width < 10 || inner.height < 2 {
        return;
    }

    let n = h.pcpu_ids.len();
    let rows = inner.height as usize;
    let budget = (inner.width as usize * 11) / 20;
    let lab_w = format!("C{}", h.pcpu_ids.iter().max().copied().unwrap_or(0)).len();
    let view = if n > 0 {
        pcpu_view(n, rows, budget, lab_w)
    } else {
        PcpuView::None
    };
    let grid_w = match view {
        PcpuView::List { width, .. } | PcpuView::Heat { width, .. } => width,
        PcpuView::None => budget.min(38),
    };

    let gx_w = inner.width.saturating_sub(grid_w as u16 + 1);
    let graph = Rect::new(inner.x, inner.y, gx_w, inner.height);
    area_graph(
        buf,
        graph,
        &app.hist.cpu.tail(gx_w as usize * 2),
        100.0,
        Paint::Height(&th.cpu),
    );

    let busy = h.cpu_busy * 100.0;
    let mut lbl = vec![
        Span::styled(" total ", Style::new().fg(th.fg).add_modifier(Modifier::BOLD)),
        bold(
            if h.cpu_estimated {
                fmt::pct(busy)
            } else {
                h.pcpu_samples.label(fmt::pct(busy))
            },
            th.cpu.at(h.cpu_busy),
        ),
    ];
    if h.cpu_estimated {
        lbl.push(dim(th, " est."));
    }
    // Sum of domain CPU time in pCPU-%, next to the host figure.
    let dom_sum: f64 = r.domains.iter().map(|d| d.cpu_pct).sum();
    lbl.push(dim(th, "   domains "));
    lbl.push(Span::styled(fmt::pct(dom_sum), Style::new().fg(th.fg)));
    lbl.push(dim(th, format!(" of {}00%", h.num_cpus)));
    // Steal: share of the CPU time vCPUs wanted that they spent waiting.
    // Only when known (the domain list shows "-" otherwise) and if it fits.
    if let Some(st) = h.steal_pct {
        let v = fmt::pct(st);
        if Line::from(lbl.clone()).width() + 9 + v.len() <= graph.width as usize {
            lbl.push(dim(th, "   steal "));
            lbl.push(Span::styled(v, Style::new().fg(steal_color(th, Some(st)))));
        }
    }
    put(buf, graph.x, graph.y, graph.width, &Line::from(lbl));

    let gx = inner.x + gx_w + 1;
    if let PcpuView::Heat {
        cols,
        cw,
        half,
        labels,
        ..
    } = view
    {
        let area = Rect::new(gx, inner.y, grid_w as u16, inner.height);
        pcpu_heatmap(buf, th, h, area, cols, cw, half, labels.then_some(lab_w));
        // The hottest pCPUs, since single cells are hard to read.
        let mut hot: Vec<(u32, f64)> = h
            .pcpu_ids
            .iter()
            .copied()
            .zip(h.pcpu_busy.iter().copied())
            .filter_map(|(id, v)| v.map(|v| (id, v)))
            .collect();
        hot.sort_by(|a, b| b.1.total_cmp(&a.1));
        let mut sp = vec![dim(th, " hottest ")];
        let mut used = 9;
        for (id, v) in hot.iter().take(6) {
            let (a, b) = (format!("C{id} "), format!("{:.0}%  ", v * 100.0));
            used += a.len() + b.len();
            if used > graph.width as usize {
                break;
            }
            sp.push(dim(th, a));
            sp.push(Span::styled(b, Style::new().fg(th.cpu.at(*v))));
        }
        if graph.height > 2 {
            put(buf, graph.x, graph.y + 1, graph.width, &Line::from(sp));
        }
    } else if let PcpuView::List { cols, gw, .. } = view {
        let rows_used = n.div_ceil(cols.max(1));
        let ew = grid_w / cols.max(1);
        for i in 0..n {
            let (col, row) = (i / rows_used, i % rows_used);
            let x = gx + (col * ew) as u16;
            let y = inner.y + row as u16;
            if y >= inner.y + inner.height {
                continue;
            }
            let v = h.pcpu_busy.get(i).copied().flatten();
            let id = h.pcpu_ids.get(i).copied().unwrap_or(i as u32);
            let mut sp = vec![dim(th, format!("{:<w$} ", format!("C{id}"), w = lab_w))];
            if gw > 0 {
                sp.extend(mini_graph(
                    &app.hist.pcpu.get(&id).map(|s| s.tail(gw * 2)).unwrap_or_default(),
                    100.0,
                    gw,
                    &th.cpu,
                ));
                sp.push(Span::raw(" "));
            }
            sp.push(Span::styled(
                v.map_or_else(|| "    -".into(), |v| format!("{:>4.0}%", v * 100.0)),
                Style::new().fg(v.map_or(th.dim, |v| th.cpu.at(v))),
            ));
            put(buf, x, y, ew.saturating_sub(2) as u16, &Line::from(sp));
        }
    } else {
        // No per-pCPU data (stock libxenstat): show the hungriest domains.
        let mut doms: Vec<&DomRates> = r.domains.iter().collect();
        doms.sort_by(|a, b| b.cpu_pct.total_cmp(&a.cpu_pct));
        let w = grid_w;
        put(
            buf,
            gx,
            inner.y,
            w as u16,
            &Line::from(dim(th, "top domains  (no per-pCPU data, see i)")),
        );
        // Sized on every domain, not just the rows drawn: these change
        // order every second.
        let room = w.saturating_sub(4 + 1 + 6 + 2);
        let longest = app
            .frame
            .name_w
            .cpu
            .hold(longest_name(r.domains.iter()), Instant::now());
        let nw = name_width(longest, room, 9);
        let mw = room.saturating_sub(nw);
        for (i, d) in doms.iter().take(rows.saturating_sub(1)).enumerate() {
            let cap = (d.vcpus_online.max(1) * 100) as f64;
            // IDs next to names: anyone who can rename a VM can call it
            // "Domain-0".
            let mut sp = vec![
                dim(th, format!("{:>3} ", d.id)),
                Span::styled(
                    format!("{} ", fmt::pad(&d.name, nw, false)),
                    Style::new().fg(th.fg),
                ),
            ];
            sp.extend(meter(d.cpu_pct / cap, mw, &th.cpu, th.meter_empty));
            sp.push(Span::styled(
                format!("{:>6}", fmt::pct(d.cpu_pct)),
                Style::new().fg(th.fg),
            ));
            put(buf, gx, inner.y + 1 + i as u16, w as u16, &Line::from(sp));
        }
    }
}

#[derive(Clone, Copy)]
enum PcpuView {
    None,
    /// One line per pCPU: label, mini history graph (`gw` cells), percent.
    List {
        width: usize,
        cols: usize,
        gw: usize,
    },
    /// One coloured cell per pCPU, `cw` columns wide; `half` packs two pCPU
    /// rows per text row with half-block glyphs.
    Heat {
        width: usize,
        cols: usize,
        cw: usize,
        half: bool,
        labels: bool,
    },
}

/// Pick the richest pCPU view that fits `max_w` x `rows`.
fn pcpu_view(n: usize, rows: usize, max_w: usize, lab_w: usize) -> PcpuView {
    let rows = rows.max(1);
    for gw in [10usize, 8, 6, 4] {
        let cols = n.div_ceil(rows);
        // label, space, graph, space, "100%", gutter
        let width = cols * (lab_w + 1 + gw + 1 + 5 + 2);
        if width <= max_w {
            return PcpuView::List { width, cols, gw };
        }
    }
    for labels in [true, false] {
        for (cw, half) in [(3, false), (2, false), (3, true), (2, true), (1, true)] {
            let cap = rows * if half { 2 } else { 1 };
            let min_cols = n.div_ceil(cap);
            // Rows of 8/16/32... read far better than rows of 10 or 13,
            // and give a squarer grid.
            let round = min_cols.div_ceil(8) * 8;
            for cols in [round.min(n), min_cols] {
                let width = if labels { lab_w + 1 } else { 0 } + cols * cw;
                if width <= max_w {
                    return PcpuView::Heat {
                        width,
                        cols,
                        cw,
                        half,
                        labels,
                    };
                }
            }
        }
    }
    // Enormous host in a tiny box: densest layout, clipped.
    let cols = n.div_ceil(rows * 2);
    PcpuView::Heat {
        width: max_w,
        cols,
        cw: 1,
        half: true,
        labels: false,
    }
}

#[allow(clippy::too_many_arguments)]
fn pcpu_heatmap(
    buf: &mut Buffer,
    th: &Theme,
    h: &crate::model::HostRates,
    area: Rect,
    cols: usize,
    cw: usize,
    half: bool,
    label_w: Option<usize>,
) {
    if th.mono {
        // No colour: shade glyphs carry the load instead, one per cell.
        pcpu_shades(buf, th, h, area, cols, cw, half, label_w);
        return;
    }
    let color = |i: usize| -> Option<Color> {
        let v = (*h.pcpu_busy.get(i)?).unwrap_or(f64::NAN);
        if !v.is_finite() {
            return Some(th.dim);
        }
        // Idle cores stay dim so the busy ones stand out.
        Some(if v < 0.02 { th.meter_empty } else { th.cpu.at(v) })
    };
    let per_row = if half { 2 } else { 1 };
    let n = h.pcpu_busy.len();
    let x0 = area.x + label_w.map(|w| w as u16 + 1).unwrap_or(0);
    // Leave a gap between cells when they're wide enough.
    let fill = if cw > 1 { cw - 1 } else { 1 };
    for ty in 0..area.height as usize {
        let first = ty * per_row * cols;
        if first >= n {
            break;
        }
        let y = area.y + ty as u16;
        if let Some(w) = label_w {
            let id = h.pcpu_ids.get(first).copied().unwrap_or(first as u32);
            buf.set_stringn(
                area.x,
                y,
                format!("{:<w$}", format!("C{id}")),
                w,
                Style::new().fg(th.dim),
            );
        }
        for c in 0..cols {
            let x = x0 + (c * cw) as u16;
            if x + fill as u16 > area.x + area.width {
                break;
            }
            let top = color(first + c);
            let bot = if half { color(first + cols + c) } else { None };
            let (sym, fg, bg) = match (top, bot, half) {
                (Some(t), Some(b), true) => ("▀", t, b),
                (Some(t), None, true) => ("▀", t, th.bg),
                // Upper half-block: the empty lower half is the gap between
                // rows, so tiles come out roughly square.
                (Some(t), _, false) => ("▀", t, th.bg),
                (None, _, _) => continue,
            };
            for k in 0..fill {
                if let Some(cell) = buf.cell_mut((x + k as u16, y)) {
                    cell.set_symbol(sym).set_fg(fg).set_bg(bg);
                }
            }
        }
    }
}

/// Monochrome heatmap: ` ░▒▓█` by load. In half-height mode a cell covers
/// two pCPUs and shows the busier one.
#[allow(clippy::too_many_arguments)]
fn pcpu_shades(
    buf: &mut Buffer,
    th: &Theme,
    h: &crate::model::HostRates,
    area: Rect,
    cols: usize,
    cw: usize,
    half: bool,
    label_w: Option<usize>,
) {
    const SHADE: [&str; 5] = ["·", "░", "▒", "▓", "█"];
    let per_row = if half { 2 } else { 1 };
    let n = h.pcpu_busy.len();
    let x0 = area.x + label_w.map(|w| w as u16 + 1).unwrap_or(0);
    let fill = if cw > 1 { cw - 1 } else { 1 };
    for ty in 0..area.height as usize {
        let first = ty * per_row * cols;
        if first >= n {
            break;
        }
        let y = area.y + ty as u16;
        if let Some(w) = label_w {
            let id = h.pcpu_ids.get(first).copied().unwrap_or(first as u32);
            buf.set_stringn(
                area.x,
                y,
                format!("{:<w$}", format!("C{id}")),
                w,
                Style::new().fg(th.dim),
            );
        }
        for c in 0..cols {
            let x = x0 + (c * cw) as u16;
            if x + fill as u16 > area.x + area.width {
                break;
            }
            let top = h.pcpu_busy.get(first + c).copied().flatten();
            let bot = if half {
                h.pcpu_busy.get(first + cols + c).copied().flatten()
            } else {
                None
            };
            let Some(v) = top.map(|t| bot.map_or(t, |b| t.max(b))).or(bot) else {
                if first + c < h.pcpu_busy.len() {
                    if let Some(cell) = buf.cell_mut((x, y)) {
                        cell.set_char('·').set_fg(th.dim);
                    }
                }
                continue;
            };
            let lvl = if v < 0.02 {
                0
            } else {
                (1.0 + v * 3.99).min(4.0) as usize
            };
            for k in 0..fill {
                if let Some(cell) = buf.cell_mut((x + k as u16, y)) {
                    cell.set_symbol(SHADE[lvl]).set_fg(th.fg);
                }
            }
        }
    }
}
