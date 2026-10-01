mod widgets;

use crate::app::{App, SortKey};
use crate::fmt;
use crate::model::{Backing, DomRates, DomState, Rates};
use crate::theme::{Gradient, Theme};
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Widget};
use ratatui::Frame;
use widgets::*;

pub fn draw(f: &mut Frame, app: &mut App) {
    let th = app.theme();
    let area = f.area();
    let buf = f.buffer_mut();
    buf.set_style(area, Style::new().bg(th.bg).fg(th.fg));
    for pos in area.positions() {
        if let Some(c) = buf.cell_mut(pos) {
            c.set_symbol(" ");
        }
    }

    let [head, body] = Layout::vertical([Constraint::Length(1), Constraint::Fill(1)]).areas(area);
    header(buf, app, head);

    let Some(rates) = app.rates.clone() else {
        let msg = match &app.error {
            Some(e) => format!("error: {e}"),
            None => "collecting first sample…".into(),
        };
        let w = msg.chars().count() as u16;
        let x = body.x + body.width.saturating_sub(w) / 2;
        buf.set_string(x, body.y + body.height / 2, msg, Style::new().fg(th.dim));
        return;
    };

    let h = body.height;
    let top_on = app.show[0] || app.show[1];
    let mid_on = app.show[2] || app.show[3];
    let (mut top_h, mut mid_h) = match h {
        50.. => (15, 13),
        40..=49 => (12, 10),
        32..=39 => (10, 8),
        24..=31 => (9, 0),
        _ => (0, 0),
    };
    if (24..32).contains(&h) && !top_on {
        mid_h = 8;
    }
    if !top_on {
        top_h = 0;
    }
    if !mid_on {
        mid_h = 0;
    }
    // Details stacked under the list need the room more than net/disk do.
    let stacked_detail = app.detail && app.selected.is_some() && area.width < 160;
    if stacked_detail && h < 45 {
        mid_h = 0;
    }
    // Few domains: hand the list's unused rows to the graph boxes.
    if !app.detail && h >= 24 {
        let want = app.visible().len() as u16 + 3;
        let spare = h.saturating_sub(top_h + mid_h).saturating_sub(want.max(8));
        let (to_top, to_mid) = match (top_h > 0, mid_h > 0) {
            (true, true) => (spare / 2, spare - spare / 2),
            (true, false) => (spare, 0),
            (false, true) => (0, spare),
            _ => (0, 0),
        };
        top_h += to_top.min(12);
        mid_h += to_mid.min(14);
    }
    let [top, mid, doms] = Layout::vertical([
        Constraint::Length(top_h),
        Constraint::Length(mid_h),
        Constraint::Fill(1),
    ])
    .areas(body);

    if top_h > 0 {
        let mem_w = (top.width * 3 / 10).clamp(30, 46);
        match (app.show[0], app.show[1]) {
            (true, true) => {
                let [c, m] = Layout::horizontal([Constraint::Fill(1), Constraint::Length(mem_w)]).areas(top);
                cpu_box(buf, app, &rates, c);
                mem_box(buf, app, &rates, m);
            }
            (true, false) => cpu_box(buf, app, &rates, top),
            (false, true) => mem_box(buf, app, &rates, top),
            _ => {}
        }
    }
    if mid_h > 0 {
        match (app.show[2], app.show[3]) {
            (true, true) => {
                let [n, d] =
                    Layout::horizontal([Constraint::Percentage(45), Constraint::Percentage(55)]).areas(mid);
                net_box(buf, app, &rates, n);
                disk_box(buf, app, &rates, d);
            }
            (true, false) => net_box(buf, app, &rates, mid),
            (false, true) => disk_box(buf, app, &rates, mid),
            _ => {}
        }
    }

    let vis = app.visible();
    let sel = app
        .selected_index(&vis)
        .and_then(|i| vis.get(i).map(|d| (*d).clone()));
    drop(vis);
    let (list_area, detail_area) = match (&sel, app.detail) {
        (Some(_), true) if doms.width >= 160 => {
            let [a, b] = Layout::horizontal([Constraint::Fill(1), Constraint::Length(74)]).areas(doms);
            (a, Some(b))
        }
        (Some(d), true) => {
            let dh = detail_height(d, doms.width).min(doms.height.saturating_sub(6));
            let [a, b] = Layout::vertical([Constraint::Fill(1), Constraint::Length(dh)]).areas(doms);
            (a, Some(b))
        }
        _ => (doms, None),
    };
    domains_box(buf, app, &rates, list_area);
    if let (Some(d), Some(a)) = (sel, detail_area) {
        detail_box(buf, app, &d, a);
    }

    if app.help {
        help_popup(buf, th, area);
    }
    if app.info {
        info_popup(buf, app, area);
    }
    if app.ansi256 {
        for pos in area.positions() {
            if let Some(c) = buf.cell_mut(pos) {
                c.fg = to_256(c.fg);
                c.bg = to_256(c.bg);
            }
        }
    }
}

/// Nearest xterm 256-colour palette entry (6x6x6 cube or grey ramp).
fn to_256(c: Color) -> Color {
    let Color::Rgb(r, g, b) = c else { return c };
    const LV: [u8; 6] = [0, 95, 135, 175, 215, 255];
    let near = |v: u8| -> usize {
        LV.iter()
            .enumerate()
            .min_by_key(|(_, &l)| (l as i32 - v as i32).abs())
            .map(|(i, _)| i)
            .unwrap_or(0)
    };
    let (ri, gi, bi) = (near(r), near(g), near(b));
    let d = |a: (u8, u8, u8)| {
        let f = |x: u8, y: u8| (x as i32 - y as i32).pow(2);
        f(a.0, r) + f(a.1, g) + f(a.2, b)
    };
    let cube = (LV[ri], LV[gi], LV[bi]);
    let avg = (r as u32 + g as u32 + b as u32) / 3;
    let gi_ = ((avg.saturating_sub(8)) / 10).min(23) as u8;
    let gv = 8 + 10 * gi_;
    if d((gv, gv, gv)) < d(cube) {
        Color::Indexed(232 + gi_)
    } else {
        Color::Indexed(16 + 36 * ri as u8 + 6 * gi as u8 + bi as u8)
    }
}

// ---------------------------------------------------------------------------
// Chrome

fn boxed<'a>(th: &Theme, num: &'a str, title: &'a str, right: Vec<Span<'a>>) -> Block<'a> {
    let mut r = vec![Span::raw(" ")];
    r.extend(right);
    r.push(Span::raw(" "));
    Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(th.border))
        .title_top(Line::from(vec![
            Span::raw(" "),
            Span::styled(num, Style::new().fg(th.key)),
            Span::styled(title, Style::new().fg(th.title).add_modifier(Modifier::BOLD)),
            Span::raw(" "),
        ]))
        .title_top(Line::from(r).right_aligned())
}

fn dim(th: &Theme, s: impl Into<String>) -> Span<'static> {
    Span::styled(s.into(), Style::new().fg(th.dim))
}

fn bold(s: impl Into<String>, c: Color) -> Span<'static> {
    Span::styled(s.into(), Style::new().fg(c).add_modifier(Modifier::BOLD))
}

fn utc_clock() -> String {
    let s = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
        % 86400;
    format!("{:02}:{:02}:{:02} UTC", s / 3600, s / 60 % 60, s % 60)
}

fn header(buf: &mut Buffer, app: &App, area: Rect) {
    let th = app.theme();
    let (host, ver) = app
        .rates
        .as_ref()
        .map(|r| (r.host.hostname.clone(), r.host.xen_version.clone()))
        .unwrap_or_default();
    let left = Line::from(vec![
        Span::styled(
            " xentop-ng ",
            Style::new().bg(th.key).fg(th.bg).add_modifier(Modifier::BOLD),
        ),
        Span::raw(" "),
        bold(host, th.title),
        dim(th, "  Xen "),
        Span::styled(ver, Style::new().fg(th.fg)),
        dim(th, format!("  {}", app.source_desc)),
    ]);
    // Subtle hint when some data is missing or rebuilt by fallbacks
    // (libxenstat patches not upstream yet). Details behind `i`.
    let mut left = left;
    if app.status.degraded() {
        left.push_span(Span::styled("  ◐ partial data", Style::new().fg(th.warn)));
        left.push_span(dim(th, " (i)"));
    } else if app.status.uses_fallback() {
        left.push_span(dim(th, "  ◐ fallback (i)"));
    }
    buf.set_line(area.x, area.y, &left, area.width);

    let mut right = Vec::new();
    if let Some(e) = &app.error {
        right.push(Span::styled(
            format!(" {} ", fmt::trunc(e, 50)),
            Style::new().fg(th.bad),
        ));
    }
    if app.paused {
        right.push(Span::styled(" ⏸ paused ", Style::new().fg(th.bg).bg(th.warn)));
        right.push(Span::raw(" "));
    }
    right.push(dim(th, "every "));
    right.push(Span::styled(
        format!("{:.2}s", app.interval.as_secs_f64()),
        Style::new().fg(th.fg),
    ));
    right.push(dim(th, "  "));
    right.push(Span::styled(utc_clock(), Style::new().fg(th.title)));
    right.push(Span::raw(" "));
    let rl = Line::from(right);
    let w = rl.width() as u16;
    if w < area.width {
        buf.set_line(area.x + area.width - w, area.y, &rl, w);
    }
}

// ---------------------------------------------------------------------------
// CPU

fn cpu_box(buf: &mut Buffer, app: &App, r: &Rates, area: Rect) {
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

    let n = app.hist.pcpu.len();
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
        bold(fmt::pct(busy), th.cpu.at(h.cpu_busy)),
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
            let v = h.pcpu_busy.get(i).copied().unwrap_or(0.0);
            let id = h.pcpu_ids.get(i).copied().unwrap_or(i as u32);
            let mut sp = vec![dim(th, format!("{:<w$} ", format!("C{id}"), w = lab_w))];
            if gw > 0 {
                sp.extend(mini_graph(&app.hist.pcpu[i].tail(gw * 2), 100.0, gw, &th.cpu));
                sp.push(Span::raw(" "));
            }
            sp.push(Span::styled(
                format!("{:>4.0}%", v * 100.0),
                Style::new().fg(th.cpu.at(v)),
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
        let mw = w.saturating_sub(14 + 6 + 2);
        for (i, d) in doms.iter().take(rows.saturating_sub(1)).enumerate() {
            let cap = (d.vcpus_online.max(1) * 100) as f64;
            // IDs next to names: anyone who can rename a VM can call it
            // "Domain-0".
            let mut sp = vec![
                dim(th, format!("{:>3} ", d.id)),
                Span::styled(
                    format!("{} ", fmt::pad(&d.name, 9, false)),
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
    let color = |i: usize| -> Option<Color> {
        let v = *h.pcpu_busy.get(i)?;
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

// ---------------------------------------------------------------------------
// Memory

fn mem_box(buf: &mut Buffer, app: &App, r: &Rates, area: Rect) {
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
    let nw = (w / 3).clamp(8, 16);
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

// ---------------------------------------------------------------------------
// Network & disk: mirrored graphs

/// Two graphs stacked in one box, each growing up from its own baseline
/// (a dim rule under the top one) and auto-scaled separately.
fn stacked(
    buf: &mut Buffer,
    th: &Theme,
    area: Rect,
    top: (&[f64], &Gradient, Line<'static>),
    bottom: (&[f64], &Gradient, Line<'static>),
    floor: f64,
) {
    if area.height < 3 {
        return;
    }
    let top_h = (area.height - 1) / 2;
    let a = Rect::new(area.x, area.y, area.width, top_h);
    let rule = area.y + top_h;
    let b = Rect::new(area.x, rule + 1, area.width, area.height - top_h - 1);
    let n = area.width as usize * 2;
    for (r, (data, grad, label)) in [(a, top), (b, bottom)] {
        let max = data.iter().rev().take(n).copied().fold(0.0, f64::max) * 1.15;
        area_graph(buf, r, data, max.max(floor), Paint::Height(grad));
        put(buf, r.x, r.y, r.width, &label);
    }
    for x in area.x..area.x + area.width {
        if let Some(c) = buf.cell_mut((x, rule)) {
            c.set_char('─').set_fg(th.meter_empty);
        }
    }
}

fn net_box(buf: &mut Buffer, app: &App, r: &Rates, area: Rect) {
    let th = app.theme();
    let h = &r.host;
    let vifs: usize = r.domains.iter().map(|d| d.nets.len()).sum();
    let block = boxed(
        th,
        "³",
        "net",
        vec![
            Span::styled(format!("{vifs}"), Style::new().fg(th.fg)),
            dim(th, " vifs"),
        ],
    );
    let inner = block.inner(area);
    block.render(area, buf);
    let w = inner.width as usize * 2;
    let (rx, tx) = (app.hist.rx.tail(w), app.hist.tx.tail(w));
    let peak = |s: &[f64]| s.iter().copied().fold(0.0, f64::max);
    let lbl = |arrow: &str, what: &str, v: f64, pk: f64, g: &Gradient| {
        Line::from(vec![
            Span::styled(format!(" {arrow} "), Style::new().fg(g.at(1.0))),
            dim(th, format!("{what} ")),
            bold(format!("{}/s", fmt::rate(v)), g.at(0.9)),
            dim(th, format!("  peak {}/s ", fmt::rate(pk))),
        ])
    };
    stacked(
        buf,
        th,
        inner,
        (&rx, &th.rx, lbl("▼", "to VMs", h.net_rx_bps, peak(&rx), &th.rx)),
        (&tx, &th.tx, lbl("▲", "from VMs", h.net_tx_bps, peak(&tx), &th.tx)),
        // Below 128 KiB/s a graph would only show noise at full height.
        128.0 * 1024.0,
    );
}

/// Latency colour on a log scale: 50µs is great, 20ms is terrible.
fn lat_color(th: &Theme, us: Option<f64>) -> Color {
    match us {
        None => th.dim,
        Some(u) => {
            let f = ((u.max(1.0) / 50.0).log10() / (20_000.0f64 / 50.0).log10()).clamp(0.0, 1.0);
            th.lat.at(f)
        }
    }
}

/// Steal colour: dim when negligible, then green to red, red from 20%.
fn steal_color(th: &Theme, pct: Option<f64>) -> Color {
    match pct {
        Some(p) if p >= 0.5 => th.cpu.at((p / 20.0).max(0.15)),
        _ => th.dim,
    }
}

fn disk_box(buf: &mut Buffer, app: &App, r: &Rates, area: Rect) {
    let th = app.theme();
    let h = &r.host;
    let vbds: usize = r.domains.iter().map(|d| d.vbds.len()).sum();
    let (n, what) = if app.sr_view {
        (r.srs.len(), if r.srs.len() == 1 { " SR" } else { " SRs" })
    } else {
        (vbds, " vbds")
    };
    let block = boxed(
        th,
        "⁴",
        "disk",
        vec![
            Span::styled(format!("{n}"), Style::new().fg(th.fg)),
            dim(th, what),
        ],
    );
    let inner = block.inner(area);
    block.render(area, buf);
    if inner.width < 10 {
        return;
    }
    if app.sr_view {
        sr_table(buf, app, r, inner);
        return;
    }
    let side_w = if inner.width >= 56 { 25 } else { 0 };
    let g = Rect::new(inner.x, inner.y, inner.width - side_w, inner.height);
    let w = g.width as usize * 2;
    let (rd, wr) = (app.hist.rd.tail(w), app.hist.wr.tail(w));
    let peak = |s: &[f64]| s.iter().copied().fold(0.0, f64::max);
    let lbl = |arrow: &str, v: f64, iops: f64, pk: f64, gr: &Gradient| {
        Line::from(vec![
            Span::styled(format!(" {arrow} "), Style::new().fg(gr.at(1.0))),
            bold(format!("{}/s", fmt::rate(v)), gr.at(0.9)),
            dim(
                th,
                format!("  {} IOPS  peak {}/s ", fmt::count(iops), fmt::rate(pk)),
            ),
        ])
    };
    stacked(
        buf,
        th,
        g,
        (
            &rd,
            &th.rd,
            lbl("R", h.disk_rd_bps, h.disk_rd_iops, peak(&rd), &th.rd),
        ),
        (
            &wr,
            &th.wr,
            lbl("W", h.disk_wr_bps, h.disk_wr_iops, peak(&wr), &th.wr),
        ),
        1024.0 * 1024.0,
    );

    if side_w == 0 {
        return;
    }
    let sx = g.x + g.width + 1;
    let sw = side_w - 1;
    let bottom = inner.y + inner.height;
    // Vertical separator.
    for y in inner.y..bottom {
        if let Some(c) = buf.cell_mut((sx - 1, y)) {
            c.set_char('│').set_fg(th.border);
        }
    }
    let mut y = inner.y;
    let mut row = |buf: &mut Buffer, l: Line| {
        if y < bottom {
            put(buf, sx, y, sw, &l);
            y += 1;
        }
    };
    row(buf, Line::from(dim(th, "latency     read  write")));
    row(
        buf,
        Line::from(vec![
            dim(th, "  avg   "),
            Span::styled(
                format!("{:>7}", fmt::lat(h.disk_rd_lat_us)),
                Style::new().fg(lat_color(th, h.disk_rd_lat_us)),
            ),
            Span::styled(
                format!("{:>7}", fmt::lat(h.disk_wr_lat_us)),
                Style::new().fg(lat_color(th, h.disk_wr_lat_us)),
            ),
        ]),
    );
    let lat_hist: Vec<f64> = app
        .hist
        .rlat
        .tail(sw as usize * 2)
        .iter()
        .zip(app.hist.wlat.tail(sw as usize * 2))
        .map(|(a, b)| a.max(b))
        .collect();
    // The graph takes all rows left under the figures, however tall the
    // box grew.
    let lat_h = bottom.saturating_sub(y);
    if lat_h > 0 {
        let mx = lat_hist.iter().copied().fold(0.0, f64::max);
        let paint = |us: f64| lat_color(th, Some(us));
        area_graph(
            buf,
            Rect::new(sx, y, sw, lat_h),
            &lat_hist,
            mx * 1.1,
            Paint::Value(&paint),
        );
        if mx > 0.0 {
            put(
                buf,
                sx,
                y,
                sw,
                &Line::from(dim(th, format!("peak {}", fmt::lat(Some(mx))))),
            );
        }
    }
}

/// Columns of the per-SR table.
#[derive(Clone, Copy, PartialEq)]
enum SrCol {
    Sr,
    Kind,
    Vbds,
    Iops,
    Read,
    Write,
    RLat,
    WLat,
    Top,
}

/// SR table columns that fit `width`, the top-VM column taking what's left.
fn sr_cols(width: usize) -> Vec<(SrCol, usize)> {
    // (column, width, priority: lower = kept longer)
    let all = [
        (SrCol::Sr, 8, 0),
        (SrCol::Kind, 4, 4),
        (SrCol::Vbds, 4, 5),
        (SrCol::Iops, 6, 0),
        (SrCol::Read, 7, 3),
        (SrCol::Write, 7, 3),
        (SrCol::RLat, 7, 2),
        (SrCol::WLat, 7, 0),
        (SrCol::Top, 10, 1),
    ];
    for max_prio in (0..=5).rev() {
        let mut cols: Vec<(SrCol, usize)> = all
            .iter()
            .filter(|c| c.2 <= max_prio)
            .map(|c| (c.0, c.1))
            .collect();
        let used: usize = cols.iter().map(|c| c.1 + 1).sum();
        if used <= width || max_prio == 0 {
            if let Some(c) = cols.iter_mut().find(|c| c.0 == SrCol::Top) {
                c.1 += width.saturating_sub(used).min(24);
            }
            return cols;
        }
    }
    Vec::new()
}

/// SR UUIDs are shown by their first block, like `xe` users abbreviate
/// them; other storage keys (directories) by their tail.
fn short_sr(key: &str, w: usize) -> String {
    if crate::source::xenstore::uuid(key).is_some() {
        return key.chars().take(8.min(w)).collect();
    }
    trunc_left(key, w)
}

/// Keep the end of `s` within `w` columns: "…/xen/images".
fn trunc_left(s: &str, w: usize) -> String {
    if fmt::width(s) <= w {
        return s.to_string();
    }
    if w == 0 {
        return String::new();
    }
    let mut tail: Vec<char> = Vec::new();
    let mut used = 1;
    for c in s.chars().rev() {
        let cw = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
        if used + cw > w {
            break;
        }
        used += cw;
        tail.push(c);
    }
    std::iter::once('…').chain(tail.into_iter().rev()).collect()
}

/// Per-SR totals in place of the disk graphs: which SR is slow, and who
/// is hammering it.
fn sr_table(buf: &mut Buffer, app: &App, r: &Rates, area: Rect) {
    let srs = &r.srs;
    let th = app.theme();
    if area.height == 0 {
        return;
    }
    if srs.is_empty() {
        let msg = "no storage mapping (xenstore); see i   v: graphs";
        put(buf, area.x, area.y, area.width, &Line::from(dim(th, msg)));
        return;
    }
    let cols = sr_cols(area.width as usize);
    let mut x = area.x;
    for (c, w) in &cols {
        let (t, right) = match c {
            SrCol::Sr => ("SR", false),
            SrCol::Kind => ("TYPE", false),
            SrCol::Vbds => ("VBDS", true),
            SrCol::Iops => ("IOPS", true),
            SrCol::Read => ("READ", true),
            SrCol::Write => ("WRITE", true),
            SrCol::RLat => ("R LAT", true),
            SrCol::WLat => ("W LAT", true),
            SrCol::Top => ("TOP VM", false),
        };
        let style = Style::new().fg(th.dim).add_modifier(Modifier::BOLD);
        buf.set_stringn(x, area.y, fmt::pad(t, *w, right), *w, style);
        x += *w as u16 + 1;
    }
    let rows = area.height as usize - 1;
    for (i, s) in srs.iter().take(rows).enumerate() {
        let mut sp: Vec<Span> = Vec::new();
        for (c, w) in &cols {
            let w = *w;
            let num = |v: String, c: Color| Span::styled(fmt::pad(&v, w, true), Style::new().fg(c));
            match c {
                SrCol::Sr => sp.push(Span::styled(
                    fmt::pad(&short_sr(&s.sr, w), w, false),
                    Style::new().fg(th.fg).add_modifier(Modifier::BOLD),
                )),
                SrCol::Kind => sp.push(dim(th, fmt::pad(s.kind.as_deref().unwrap_or("-"), w, false))),
                SrCol::Vbds => sp.push(dim(th, fmt::pad(&s.vbds.to_string(), w, true))),
                SrCol::Iops => {
                    let v = s.iops();
                    sp.push(num(fmt::count(v), if v < 0.5 { th.dim } else { th.fg }));
                }
                SrCol::Read => sp.push(num(fmt::rate(s.rd_bps), th.rd.at(1.0))),
                SrCol::Write => sp.push(num(fmt::rate(s.wr_bps), th.wr.at(1.0))),
                SrCol::RLat => sp.push(num(fmt::lat(s.rd_lat_us), lat_color(th, s.rd_lat_us))),
                SrCol::WLat => sp.push(num(fmt::lat(s.wr_lat_us), lat_color(th, s.wr_lat_us))),
                SrCol::Top => match s.top_id.filter(|_| s.top_iops >= 0.5) {
                    // Share of the SR's IOPS, when there is room for it.
                    Some(id) => {
                        let share = s.top_iops / s.iops().max(1e-9) * 100.0;
                        sp.extend(vm_cell(
                            th,
                            id,
                            s.top_name.as_deref().unwrap_or("?"),
                            Some(share),
                            w,
                        ));
                    }
                    None => sp.push(dim(th, fmt::pad("-", w, false))),
                },
            }
            sp.push(Span::raw(" "));
        }
        put(buf, area.x, area.y + 1 + i as u16, area.width, &Line::from(sp));
    }

    // Room left: the busiest disks across all SRs, in the same columns.
    let mut y = area.y + 1 + srs.len().min(rows) as u16;
    let bottom = area.y + area.height;
    if y + 3 > bottom {
        return;
    }
    put(
        buf,
        area.x,
        y,
        area.width,
        &Line::from(dim(
            th,
            format!("{:─<w$}", "── busiest disks ", w = area.width as usize),
        )),
    );
    y += 1;
    let mut vbds: Vec<(&DomRates, &crate::model::VbdRates)> = r
        .domains
        .iter()
        .flat_map(|d| d.vbds.iter().map(move |v| (d, v)))
        .filter(|(_, v)| v.backing.group().is_some() && v.rd_iops + v.wr_iops >= 0.5)
        .collect();
    vbds.sort_by(|a, b| (b.1.rd_iops + b.1.wr_iops).total_cmp(&(a.1.rd_iops + a.1.wr_iops)));
    for (d, v) in vbds.into_iter().take((bottom - y) as usize) {
        let mut sp: Vec<Span> = Vec::new();
        for (c, w) in &cols {
            let w = *w;
            let num = |s: String, c: Color| Span::styled(fmt::pad(&s, w, true), Style::new().fg(c));
            match c {
                SrCol::Sr => {
                    let key = v.backing.group().unwrap_or_default();
                    sp.push(dim(th, fmt::pad(&short_sr(&key, w), w, false)));
                }
                SrCol::Kind => sp.push(Span::styled(fmt::pad(&v.name, w, false), Style::new().fg(th.fg))),
                SrCol::Vbds => sp.push(Span::raw(" ".repeat(w))),
                SrCol::Iops => sp.push(num(fmt::count(v.rd_iops + v.wr_iops), th.fg)),
                SrCol::Read => sp.push(num(fmt::rate(v.rd_bps), th.rd.at(1.0))),
                SrCol::Write => sp.push(num(fmt::rate(v.wr_bps), th.wr.at(1.0))),
                SrCol::RLat => sp.push(num(fmt::lat(v.rd_lat_us), lat_color(th, v.rd_lat_us))),
                SrCol::WLat => sp.push(num(fmt::lat(v.wr_lat_us), lat_color(th, v.wr_lat_us))),
                SrCol::Top => sp.extend(vm_cell(th, d.id, &d.name, None, w)),
            }
            sp.push(Span::raw(" "));
        }
        put(buf, area.x, y, area.width, &Line::from(sp));
        y += 1;
    }
}

/// "id name", plus a share in % when given and there is room, in `w`
/// columns. IDs next to names: anyone who can rename a VM can call it
/// "Domain-0".
fn vm_cell(th: &Theme, id: u32, name: &str, share: Option<f64>, w: usize) -> Vec<Span<'static>> {
    let share = match share {
        Some(p) if w >= 16 => format!(" {p:>3.0}%"),
        _ => String::new(),
    };
    let idw = format!("{id:>3} ");
    let nw = w.saturating_sub(idw.len() + share.len());
    vec![
        dim(th, idw),
        Span::styled(fmt::pad(name, nw, false), Style::new().fg(th.fg)),
        dim(th, share),
    ]
}

// ---------------------------------------------------------------------------
// Domain list

#[derive(Clone, Copy, PartialEq)]
enum Col {
    Id,
    Name,
    State,
    Vcpu,
    Cpu,
    CpuHist,
    Steal,
    Mem,
    NetRx,
    NetTx,
    DiskRd,
    DiskWr,
    Iops,
    Lat,
}

impl Col {
    fn title(self) -> &'static str {
        match self {
            Col::Id => "ID",
            Col::Name => "NAME",
            Col::State => "STATE",
            Col::Vcpu => "VCPU",
            Col::Cpu => "CPU",
            Col::CpuHist => "CPU HISTORY",
            Col::Steal => "STEAL",
            Col::Mem => "MEM",
            Col::NetRx => "NET ▼",
            Col::NetTx => "NET ▲",
            Col::DiskRd => "READ",
            Col::DiskWr => "WRITE",
            Col::Iops => "IOPS",
            Col::Lat => "LAT",
        }
    }
    fn sort(self) -> Option<SortKey> {
        Some(match self {
            Col::Id => SortKey::Id,
            Col::Name => SortKey::Name,
            Col::Cpu | Col::CpuHist => SortKey::Cpu,
            Col::Mem => SortKey::Mem,
            Col::NetRx => SortKey::Net,
            Col::DiskRd => SortKey::Disk,
            Col::Lat => SortKey::Lat,
            Col::Steal => SortKey::Steal,
            _ => return None,
        })
    }
    fn right(self) -> bool {
        !matches!(self, Col::Name | Col::State | Col::Cpu | Col::CpuHist)
    }
}

/// Choose columns and widths for the available width.
fn layout_cols(width: u16) -> Vec<(Col, usize)> {
    let width = width as usize;
    // (column, width, priority: lower = kept longer)
    let all = [
        (Col::Id, 4, 0),
        (Col::Name, 14, 0),
        (Col::State, 6, 6),
        (Col::Vcpu, 5, 7),
        (Col::Cpu, 16, 0),
        (Col::CpuHist, 12, 5),
        (Col::Steal, 6, 4),
        (Col::Mem, 7, 0),
        (Col::NetRx, 8, 1),
        (Col::NetTx, 8, 1),
        (Col::DiskRd, 8, 2),
        (Col::DiskWr, 8, 2),
        (Col::Iops, 7, 4),
        (Col::Lat, 8, 3),
    ];
    let mut max_prio = 8;
    loop {
        let cols: Vec<(Col, usize)> = all
            .iter()
            .filter(|c| c.2 <= max_prio)
            .map(|c| (c.0, c.1))
            .collect();
        let used: usize = cols.iter().map(|c| c.1 + 1).sum();
        if used <= width || max_prio == 0 {
            let mut cols = cols;
            let mut extra = width.saturating_sub(used);
            // Grow the name a bit, then give everything else to the history.
            if let Some(c) = cols.iter_mut().find(|c| c.0 == Col::Name) {
                let g = extra.min(12);
                c.1 += g;
                extra -= g;
            }
            if let Some(c) = cols.iter_mut().find(|c| c.0 == Col::CpuHist) {
                c.1 += extra;
            } else if let Some(c) = cols.iter_mut().find(|c| c.0 == Col::Name) {
                c.1 += extra;
            }
            return cols;
        }
        max_prio -= 1;
    }
}

fn pad(s: String, w: usize, right: bool) -> String {
    fmt::pad(&s, w, right)
}

fn domains_box(buf: &mut Buffer, app: &mut App, r: &Rates, area: Rect) {
    let th = app.theme();
    let vis_ids: Vec<u32> = app.visible().iter().map(|d| d.id).collect();
    let arrow = if app.reverse { "▲" } else { "▼" };
    let mut right = vec![
        Span::styled(format!("{}", vis_ids.len()), Style::new().fg(th.fg)),
        dim(
            th,
            if app.filter.is_empty() {
                " domains"
            } else {
                " matching"
            },
        ),
        dim(th, "  sort "),
        Span::styled(format!("{} {arrow}", app.sort.label()), Style::new().fg(th.key)),
    ];
    if !app.filter.is_empty() && !app.filter_edit {
        right.push(dim(th, "  filter "));
        right.push(Span::styled(
            format!("\"{}\"", app.filter),
            Style::new().fg(th.warn),
        ));
    }
    let mut block = boxed(th, "⁵", "domains", right);
    let hints: Vec<Span> = if app.filter_edit {
        vec![
            Span::styled(" filter: ", Style::new().fg(th.key)),
            Span::styled(format!("{}█ ", app.filter), Style::new().fg(th.fg)),
            dim(th, "⏎ apply  esc clear "),
        ]
    } else {
        let k = |key: &'static str, what: &'static str| {
            [Span::styled(key, Style::new().fg(th.key)), dim(th, what)]
        };
        let mut v = vec![Span::raw(" ")];
        for (key, what) in [
            ("↑↓", " select  "),
            ("⏎", " details  "),
            ("s", " sort  "),
            ("r", " reverse  "),
            ("/", " filter  "),
            ("1-5", " boxes  "),
            ("t", " theme  "),
            ("+-", " speed  "),
            ("p", " pause  "),
            ("?", " help  "),
            ("q", " quit "),
        ] {
            v.extend(k(key, what));
        }
        v
    };
    block = block.title_bottom(Line::from(hints));
    let inner = block.inner(area);
    block.render(area, buf);
    if inner.height < 2 {
        return;
    }

    let cols = layout_cols(inner.width);
    // Header
    let mut x = inner.x;
    for (c, w) in &cols {
        let active = c.sort() == Some(app.sort) && *c != Col::CpuHist;
        let t = if active {
            format!("{}{arrow}", c.title())
        } else {
            c.title().to_string()
        };
        let style = if active {
            Style::new().fg(th.key).add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(th.dim).add_modifier(Modifier::BOLD)
        };
        buf.set_stringn(x, inner.y, pad(t, *w, c.right()), *w, style);
        x += *w as u16 + 1;
    }

    let rows = Rect::new(inner.x, inner.y + 1, inner.width, inner.height - 1);
    let n_rows = rows.height as usize;
    let sel_idx = app.selected.and_then(|id| vis_ids.iter().position(|&x| x == id));
    let mut off = app.table_offset.min(vis_ids.len().saturating_sub(n_rows));
    if let Some(s) = sel_idx {
        if s < off {
            off = s;
        } else if s >= off + n_rows {
            off = s + 1 - n_rows;
        }
    }
    app.table_offset = off;
    app.table_rows = rows;

    let by_id: std::collections::HashMap<u32, &DomRates> = r.domains.iter().map(|d| (d.id, d)).collect();
    for (i, id) in vis_ids.iter().skip(off).take(n_rows).enumerate() {
        let Some(d) = by_id.get(id) else { continue };
        let y = rows.y + i as u16;
        let line = dom_row(app, th, d, &cols);
        put(buf, rows.x, y, rows.width, &line);
        if Some(off + i) == sel_idx {
            buf.set_style(Rect::new(rows.x, y, rows.width, 1), Style::new().bg(th.sel_bg));
        }
    }
    if vis_ids.is_empty() {
        buf.set_string(rows.x + 1, rows.y, "no domains match", Style::new().fg(th.dim));
    }
    // Scroll indicator.
    if vis_ids.len() > n_rows {
        let x = area.x + area.width - 1;
        let bar_h = ((n_rows * n_rows) / vis_ids.len()).max(1);
        let pos = off * (n_rows - bar_h) / (vis_ids.len() - n_rows).max(1);
        for k in 0..bar_h {
            if let Some(c) = buf.cell_mut((x, rows.y + (pos + k) as u16)) {
                c.set_char('┃').set_fg(th.dim);
            }
        }
    }
}

fn dom_row(app: &App, th: &Theme, d: &DomRates, cols: &[(Col, usize)]) -> Line<'static> {
    let hist = app.hist.doms.get(&d.id);
    let mut sp: Vec<Span<'static>> = Vec::new();
    let num = |v: String, zero: bool, w: usize, c: Color| {
        Span::styled(pad(v, w, true), Style::new().fg(if zero { th.dim } else { c }))
    };
    for (c, w) in cols {
        let w = *w;
        match c {
            Col::Id => sp.push(Span::styled(
                pad(d.id.to_string(), w, true),
                Style::new().fg(th.dim),
            )),
            Col::Name => {
                let st = match d.state {
                    _ if d.id == 0 => Style::new().fg(th.title).add_modifier(Modifier::BOLD),
                    Some(DomState::Crashed) => Style::new().fg(th.bad).add_modifier(Modifier::BOLD),
                    Some(DomState::Paused) | Some(DomState::Shutdown) | Some(DomState::Dying) => {
                        Style::new().fg(th.dim).add_modifier(Modifier::ITALIC)
                    }
                    _ => Style::new().fg(th.fg),
                };
                sp.push(Span::styled(pad(d.name.clone(), w, false), st));
            }
            Col::State => {
                // Xen's running/blocked flags are an instantaneous snapshot
                // (a busy VM is usually "blocked" at any given moment), so
                // judge activity by CPU use over the interval instead.
                let active = d.cpu_pct >= 5.0;
                let (t, c) = match d.state {
                    Some(DomState::Running) | Some(DomState::Blocked) if active => ("● run", th.ok),
                    Some(DomState::Running) | Some(DomState::Blocked) => ("○ idle", th.dim),
                    Some(DomState::Paused) => ("‖ paus", th.warn),
                    Some(DomState::Crashed) => ("✖ CRSH", th.bad),
                    Some(DomState::Shutdown) => ("◌ shut", th.dim),
                    Some(DomState::Dying) => ("◌ dyin", th.dim),
                    None => ("?", th.dim),
                };
                sp.push(Span::styled(pad(t.into(), w, false), Style::new().fg(c)));
            }
            Col::Vcpu => {
                // xenstat reports max vCPUs; show online/max when they differ.
                let max = d.vcpu_pct.len();
                let t = if d.vcpus_online == max {
                    max.to_string()
                } else {
                    format!("{}/{max}", d.vcpus_online)
                };
                sp.push(num(t, false, w, th.fg))
            }
            Col::Cpu => {
                let cap = (d.vcpus_online.max(1) * 100) as f64;
                let f = d.cpu_pct / cap;
                let mw = w.saturating_sub(7);
                sp.extend(meter(f, mw, &th.cpu, th.meter_empty));
                sp.push(Span::styled(
                    format!("{:>7}", fmt::pct(d.cpu_pct)),
                    Style::new().fg(if d.cpu_pct < 0.5 {
                        th.dim
                    } else {
                        th.cpu.at(f.max(0.15))
                    }),
                ));
            }
            Col::CpuHist => {
                let data = hist.map(|h| h.cpu.tail(w)).unwrap_or_default();
                let cap = (d.vcpus_online.max(1) * 100) as f64;
                sp.extend(sparkline(&data, cap, w, &th.cpu, th.meter_empty));
            }
            Col::Mem => sp.push(num(fmt::bytes(d.mem as f64), false, w, th.mem.at(0.6))),
            Col::NetRx => sp.push(num(fmt::rate(d.net_rx_bps), d.net_rx_bps < 0.5, w, th.rx.at(1.0))),
            Col::NetTx => sp.push(num(fmt::rate(d.net_tx_bps), d.net_tx_bps < 0.5, w, th.tx.at(1.0))),
            Col::DiskRd => sp.push(num(
                fmt::rate(d.disk_rd_bps),
                d.disk_rd_bps < 0.5,
                w,
                th.rd.at(1.0),
            )),
            Col::DiskWr => sp.push(num(
                fmt::rate(d.disk_wr_bps),
                d.disk_wr_bps < 0.5,
                w,
                th.wr.at(1.0),
            )),
            Col::Iops => {
                let v = d.disk_rd_iops + d.disk_wr_iops;
                sp.push(num(fmt::count(v), v < 0.5, w, th.fg))
            }
            Col::Lat => {
                let l = d.lat_us();
                sp.push(Span::styled(
                    pad(fmt::lat(l), w, true),
                    Style::new().fg(lat_color(th, l)),
                ));
            }
            Col::Steal => sp.push(Span::styled(
                pad(d.steal_pct.map(fmt::pct).unwrap_or_else(|| "-".into()), w, true),
                Style::new().fg(steal_color(th, d.steal_pct)),
            )),
        }
        sp.push(Span::raw(" "));
    }
    Line::from(sp)
}

// ---------------------------------------------------------------------------
// Domain detail

/// Width of one vCPU cell in the detail panel: label, meter, load, and
/// steal when the hypervisor reports it per vCPU.
fn vcpu_cell_w(d: &DomRates) -> usize {
    if d.vcpu_steal_pct.iter().any(Option::is_some) {
        31
    } else {
        23
    }
}

/// Rows the detail panel wants when stacked (borders included).
fn detail_height(d: &DomRates, width: u16) -> u16 {
    let per_row = ((width as usize).saturating_sub(2) / vcpu_cell_w(d)).max(1);
    let section = |n: usize| if n > 0 { 2 + n } else { 0 };
    // One more line for the VM UUID, and one under each mapped disk.
    let extra =
        d.vm_uuid.is_some() as usize + d.vbds.iter().filter(|v| backing_line_wanted(&v.backing)).count();
    (2 + 1 + 3 + extra + d.vcpu_pct.len().div_ceil(per_row) + section(d.vbds.len()) + section(d.nets.len()))
        as u16
}

fn backing_line_wanted(b: &Backing) -> bool {
    b.sr.is_some() || b.vdi.is_some() || b.path.is_some()
}

/// Balloon target, when it is meaningfully away from current memory (a
/// few MiB of difference is just accounting noise).
fn balloon_target(d: &DomRates) -> Option<u64> {
    let t = d.mem_target?;
    let diff = t.abs_diff(d.mem);
    (diff > 32 << 20 && diff * 50 > t.max(d.mem)).then_some(t)
}

/// The line under a disk row saying what backs it.
fn backing_line(th: &Theme, b: &Backing, w: usize) -> Line<'static> {
    let mut sp = vec![dim(th, "      └ ")];
    if b.sr.is_some() || b.vdi.is_some() {
        sp.push(dim(th, "sr "));
        sp.push(Span::styled(
            b.sr.as_deref().map(|s| short_sr(s, 8)).unwrap_or("?".into()),
            Style::new().fg(th.fg),
        ));
        if let Some(k) = &b.sr_kind {
            sp.push(dim(th, format!(" {k}")));
        }
        if let Some(v) = &b.vdi {
            sp.push(dim(th, "  vdi "));
            sp.push(Span::styled(v.clone(), Style::new().fg(th.fg)));
        } else if let Some(p) = &b.path {
            sp.push(dim(th, "  "));
            sp.push(Span::styled(
                trunc_left(p, w.saturating_sub(30)),
                Style::new().fg(th.fg),
            ));
        }
    } else if let Some(p) = &b.path {
        sp.push(Span::styled(
            trunc_left(p, w.saturating_sub(8)),
            Style::new().fg(th.fg),
        ));
    }
    Line::from(sp)
}

fn detail_box(buf: &mut Buffer, app: &App, d: &DomRates, area: Rect) {
    let th = app.theme();
    let state = match d.state {
        // Same rule as the list: activity over the interval, not the
        // instantaneous running/blocked flag.
        Some(DomState::Running) | Some(DomState::Blocked) if d.cpu_pct >= 5.0 => "running",
        Some(DomState::Running) | Some(DomState::Blocked) => "idle",
        Some(s) => s.label(),
        None => "?",
    };
    let block = boxed(
        th,
        "▸ ",
        &d.name,
        vec![
            dim(th, format!("id {}  ", d.id)),
            Span::styled(state, Style::new().fg(th.fg)),
            dim(th, "  esc close"),
        ],
    )
    .border_style(Style::new().fg(th.key));
    let inner = block.inner(area);
    block.render(area, buf);
    let w = inner.width as usize;
    if w < 20 || inner.height < 3 {
        return;
    }
    let bottom = inner.y + inner.height;
    let mut y = inner.y;
    let hist = app.hist.doms.get(&d.id);
    macro_rules! line {
        ($l:expr) => {
            if y < bottom {
                put(buf, inner.x, y, inner.width, &$l);
                y += 1;
            }
        };
    }

    // Summary.
    line!(Line::from(vec![
        dim(th, "cpu "),
        bold(
            fmt::pct(d.cpu_pct),
            th.cpu.at(d.cpu_pct / (d.vcpus_online.max(1) * 100) as f64)
        ),
        dim(
            th,
            format!(" of {}/{} vCPU online   mem ", d.vcpus_online, d.vcpu_pct.len())
        ),
        bold(fmt::bytes(d.mem as f64), th.mem.at(0.7)),
        dim(th, format!(" / {}", fmt::bytes(d.max_mem as f64))),
        match balloon_target(d) {
            Some(t) => Span::styled(
                format!(" (balloon target {})", fmt::bytes(t as f64)),
                Style::new().fg(th.warn)
            ),
            None => Span::raw(""),
        },
        dim(th, "   steal "),
        Span::styled(
            d.steal_pct.map(fmt::pct).unwrap_or_else(|| "-".into()),
            Style::new().fg(steal_color(th, d.steal_pct)),
        ),
    ]));
    if let Some(u) = &d.vm_uuid {
        line!(Line::from(vec![
            dim(th, "vm uuid "),
            Span::styled(u.clone(), Style::new().fg(th.fg)),
        ]));
    }

    // CPU history graph.
    let gh = if inner.height >= 30 { 6 } else { 3 };
    if y + gh <= bottom {
        let data = hist.map(|h| h.cpu.tail(w * 2)).unwrap_or_default();
        let cap = (d.vcpus_online.max(1) * 100) as f64;
        area_graph(
            buf,
            Rect::new(inner.x, y, inner.width, gh),
            &data,
            cap,
            Paint::Height(&th.cpu),
        );
        y += gh;
    }

    // vCPUs, with their steal time when known.
    let cw = vcpu_cell_w(d);
    let with_steal = cw > 23;
    let per_row = (w / cw).max(1);
    for chunk in d.vcpu_pct.chunks(per_row).enumerate() {
        let (ci, vals) = chunk;
        let mut sp = Vec::new();
        for (k, v) in vals.iter().enumerate() {
            let i = ci * per_row + k;
            sp.push(dim(th, format!("v{i:<2} ")));
            sp.extend(meter(v / 100.0, 12, &th.cpu, th.meter_empty));
            sp.push(Span::styled(
                format!("{:>5.0}%", v),
                Style::new().fg(th.cpu.at(v / 100.0)),
            ));
            if with_steal {
                let st = d.vcpu_steal_pct.get(i).copied().flatten();
                sp.push(dim(th, " st"));
                sp.push(Span::styled(
                    format!("{:>5}", st.map(fmt::pct).unwrap_or_else(|| "-".into())),
                    Style::new().fg(steal_color(th, st)),
                ));
            }
            sp.push(Span::raw("  "));
        }
        line!(Line::from(sp));
    }

    // Disks.
    if !d.vbds.is_empty() && y + 2 <= bottom {
        line!(Line::from(dim(th, format!("{:─<w$}", "── disks "))));
        line!(Line::from(Span::styled(
            format!(
                "{:<6}{:<9}{:>7}{:>7}{:>8}{:>8}{:>8}{:>8}{:>5}",
                "dev", "backend", "r/s", "w/s", "read", "write", "r lat", "w lat", "err"
            ),
            Style::new().fg(th.dim).add_modifier(Modifier::BOLD),
        )));
        for v in &d.vbds {
            line!(Line::from(vec![
                Span::styled(format!("{:<6}", v.name), Style::new().fg(th.fg)),
                dim(th, format!("{:<9}", v.kind.map(|k| k.label()).unwrap_or("?"))),
                Span::styled(
                    format!("{:>7}", fmt::count(v.rd_iops)),
                    Style::new().fg(th.rd.at(1.0))
                ),
                Span::styled(
                    format!("{:>7}", fmt::count(v.wr_iops)),
                    Style::new().fg(th.wr.at(1.0))
                ),
                Span::styled(
                    format!("{:>8}", fmt::rate(v.rd_bps)),
                    Style::new().fg(th.rd.at(1.0))
                ),
                Span::styled(
                    format!("{:>8}", fmt::rate(v.wr_bps)),
                    Style::new().fg(th.wr.at(1.0))
                ),
                Span::styled(
                    format!("{:>8}", fmt::lat(v.rd_lat_us)),
                    Style::new().fg(lat_color(th, v.rd_lat_us))
                ),
                Span::styled(
                    format!("{:>8}", fmt::lat(v.wr_lat_us)),
                    Style::new().fg(lat_color(th, v.wr_lat_us))
                ),
                Span::styled(
                    format!("{:>5}", v.errors),
                    Style::new().fg(if v.errors > 0 { th.bad } else { th.dim }),
                ),
            ]));
            if backing_line_wanted(&v.backing) {
                line!(backing_line(th, &v.backing, w));
            }
        }
    }

    // Network.
    if !d.nets.is_empty() && y + 2 <= bottom {
        line!(Line::from(dim(th, format!("{:─<w$}", "── network "))));
        line!(Line::from(Span::styled(
            format!(
                "{:<9}{:>9}{:>9}{:>9}{:>9}{:>8}",
                "vif", "rx", "tx", "rx pps", "tx pps", "err/drp"
            ),
            Style::new().fg(th.dim).add_modifier(Modifier::BOLD),
        )));
        for n in &d.nets {
            line!(Line::from(vec![
                Span::styled(
                    format!("{:<9}", format!("vif{}.{}", d.id, n.id)),
                    Style::new().fg(th.fg)
                ),
                Span::styled(
                    format!("{:>9}", format!("{}/s", fmt::rate(n.rx_bps))),
                    Style::new().fg(th.rx.at(1.0))
                ),
                Span::styled(
                    format!("{:>9}", format!("{}/s", fmt::rate(n.tx_bps))),
                    Style::new().fg(th.tx.at(1.0))
                ),
                Span::styled(format!("{:>9}", fmt::count(n.rx_pps)), Style::new().fg(th.fg)),
                Span::styled(format!("{:>9}", fmt::count(n.tx_pps)), Style::new().fg(th.fg)),
                Span::styled(
                    format!("{:>8}", format!("{}/{}", n.errs, n.drops)),
                    Style::new().fg(if n.errs + n.drops > 0 { th.warn } else { th.dim }),
                ),
            ]));
        }
    }
}

// ---------------------------------------------------------------------------
// Help

fn help_popup(buf: &mut Buffer, th: &Theme, area: Rect) {
    let keys: &[(&str, &str)] = &[
        ("↑ ↓  j k", "select domain"),
        ("PgUp PgDn  g G", "page / first / last"),
        ("⏎  space  dbl-click", "toggle domain details"),
        ("s  S  ← →", "next / previous sort column"),
        ("c m n d l", "sort by cpu, mem, net, disk, latency"),
        ("r", "reverse sort order"),
        ("0", "pin Domain-0 on top"),
        ("/  f", "filter by name or id"),
        ("1 2 3 4", "toggle cpu / mem / net / disk boxes"),
        ("5", "domains only (again: restore boxes)"),
        ("v", "disk box: graphs / per-SR totals"),
        ("+  -", "slower / faster refresh"),
        ("p", "pause sampling"),
        ("t  T", "cycle colour theme"),
        ("i", "data sources (what libxenstat provides)"),
        ("esc", "close details / clear filter"),
        ("q  ctrl-c", "quit"),
    ];
    let lines = keys
        .iter()
        .map(|(k, what)| {
            Line::from(vec![
                Span::styled(format!("{k:<22}"), Style::new().fg(th.key)),
                Span::styled(*what, Style::new().fg(th.fg)),
            ])
        })
        .collect();
    popup(buf, th, area, " help ", 62, lines);
}

/// Centered, bordered popup listing `lines`.
fn popup(buf: &mut Buffer, th: &Theme, area: Rect, title: &'static str, width: u16, lines: Vec<Line>) {
    let w = width.min(area.width);
    let h = (lines.len() as u16 + 4).min(area.height);
    let r = Rect::new(
        area.x + (area.width - w) / 2,
        area.y + (area.height - h) / 2,
        w,
        h,
    );
    Clear.render(r, buf);
    buf.set_style(r, Style::new().bg(th.bg));
    let block = Block::bordered()
        .border_type(BorderType::Rounded)
        .border_style(Style::new().fg(th.key))
        .title_top(Line::from(vec![bold(title, th.title)]))
        .title_bottom(Line::from(dim(th, " any key to close ")).right_aligned());
    let inner = block.inner(r);
    block.render(r, buf);
    for (i, l) in lines.iter().enumerate() {
        let y = inner.y + 1 + i as u16;
        if y >= inner.y + inner.height {
            break;
        }
        put(buf, inner.x + 1, y, inner.width.saturating_sub(2), l);
    }
}

fn info_popup(buf: &mut Buffer, app: &App, area: Rect) {
    use crate::source::Avail;
    let th = app.theme();
    let st = &app.status;
    let row =
        |what: &'static str, a: Avail, fallback: &'static str, missing: &'static str, na: &'static str| {
            let (mark, c, how) = match a {
                Avail::Lib => ("✓", th.ok, "libxenstat"),
                Avail::Fallback => ("◐", th.warn, fallback),
                Avail::Missing => ("✗", th.bad, missing),
                Avail::NotApplicable => ("·", th.dim, na),
            };
            Line::from(vec![
                Span::styled(format!("{mark} "), Style::new().fg(c)),
                Span::styled(format!("{what:<20}"), Style::new().fg(th.fg)),
                Span::styled(how, Style::new().fg(if a == Avail::Lib { th.dim } else { c })),
            ])
        };
    let lines = vec![
        Line::from(vec![
            dim(th, "library  "),
            Span::styled(app.source_desc.clone(), Style::new().fg(th.fg)),
        ]),
        Line::from(""),
        row(
            "per-pCPU load",
            st.pcpu,
            "fallback: libxenctrl xc_getcpuinfo()",
            "missing: host CPU estimated from domains",
            "n/a",
        ),
        row(
            "disk latency",
            st.vbd_latency,
            "fallback: tapdisk3 stats in /dev/shm",
            "missing: no tapdisk3 stats readable",
            "n/a: no tapdisk3 disks",
        ),
        row(
            "network (VIFs)",
            st.vifs,
            "fallback: /proc/net/dev",
            "missing",
            "n/a",
        ),
        // Never in libxenstat, so "from xenstore" is the normal case.
        {
            let (mark, c, how) = match st.storage {
                Avail::Lib | Avail::Fallback => ("✓", th.ok, "xenstore (VBD backend params, /vm)"),
                Avail::Missing => ("✗", th.bad, "missing: xenstore unreadable or unmapped"),
                Avail::NotApplicable => ("·", th.dim, "n/a: no disks"),
            };
            Line::from(vec![
                Span::styled(format!("{mark} "), Style::new().fg(c)),
                Span::styled(format!("{:<20}", "SR/VDI, VM UUIDs"), Style::new().fg(th.fg)),
                Span::styled(how, Style::new().fg(if c == th.ok { th.dim } else { c })),
            ])
        },
        row(
            "steal time",
            st.steal,
            "fallback: XCP-ng domain runstate, no vCPU",
            "missing: needs hypervisor patch (0003)",
            "n/a",
        ),
        Line::from(""),
        Line::from(dim(th, "Fallbacks fill in what this libxenstat lacks. The")),
        Line::from(dim(th, "libxenstat patches in the xentop-ng repository")),
        Line::from(dim(th, "(libxenstat/) provide all of it natively; steal")),
        Line::from(dim(th, "time per vCPU also needs their hypervisor patch.")),
    ];
    popup(buf, th, area, " data sources ", 70, lines);
}

#[cfg(test)]
mod tests {
    use crate::app::App;
    use crate::source::demo::{DemoConfig, DemoSource};
    use crate::source::Source;
    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event::{KeyCode, KeyEvent};
    use ratatui::Terminal;
    use std::time::Duration;

    fn app(cfg: &DemoConfig) -> App {
        let mut src = DemoSource::new(cfg);
        let hist = src.warmup(40);
        let mut app = App::new(Box::new(src), Duration::from_secs(1), 0);
        for s in hist {
            app.ingest(s);
        }
        app.status = app.source.status();
        app
    }

    /// Every view, at every size from absurdly small to very large, on hosts
    /// from 1 to 1024 pCPUs, must render without panicking.
    #[test]
    fn renders_everywhere() {
        let gib = 1u64 << 30;
        let hosts = [
            (1, gib, false),
            (4, 16 * gib, true),
            (16, 128 * gib, false),
            (128, 1024 * gib, false),
            (1024, 8192 * gib, false),
        ];
        let sizes = [1u16, 5, 12, 20, 24, 31, 40, 57, 80, 119, 160, 200, 300];
        for (pcpus, mem, stock) in hosts {
            let cfg = DemoConfig {
                pcpus,
                mem,
                stock,
                ..Default::default()
            };
            let mut a = app(&cfg);
            for &w in &sizes {
                for &h in sizes.iter().filter(|&&h| h <= 80) {
                    let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
                    // Graphs, SR table, SR table + details (+ popups),
                    // graphs + details.
                    for keys in [
                        &[][..],
                        &[KeyCode::Char('v')],
                        &[KeyCode::Down, KeyCode::Enter],
                        &[KeyCode::Char('?')],
                        &[KeyCode::Char('i')],
                        &[KeyCode::Char('v')],
                    ] {
                        for k in keys {
                            a.on_key(KeyEvent::from(*k));
                        }
                        term.draw(|f| super::draw(f, &mut a)).unwrap();
                        a.help = false;
                        a.info = false;
                    }
                    a.detail = false;
                    a.sr_view = false;
                }
            }
        }
    }

    /// The SR view with nothing to show (no xenstore), and with storage
    /// keys that aren't SR UUIDs (plain Xen: backing directories).
    #[test]
    fn sr_view_edge_cases() {
        let mut a = app(&DemoConfig::default());
        a.sr_view = true;
        let r = a.rates.as_mut().unwrap();
        r.srs.truncate(2);
        r.srs[0].sr = "/very/long/path/to/some/xen/images/directory".into();
        r.srs[1].top_id = Some(123456);
        r.srs[1].top_name = Some("宽字符名".repeat(10));
        for (w, h) in [(40, 30), (80, 40), (200, 60)] {
            let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
            term.draw(|f| super::draw(f, &mut a)).unwrap();
        }
        a.rates.as_mut().unwrap().srs.clear();
        let mut term = Terminal::new(TestBackend::new(120, 40)).unwrap();
        term.draw(|f| super::draw(f, &mut a)).unwrap();
        assert_eq!(
            super::short_sr("ac70e429-0dec-3ccd-1d24-2713c6104b65", 8),
            "ac70e429"
        );
        assert_eq!(super::trunc_left("/var/lib/xen/images", 8), "…/images");
        assert_eq!(super::trunc_left("/srv", 8), "/srv");
    }
}
