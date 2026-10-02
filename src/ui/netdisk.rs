//! The net and disk boxes: mirrored graphs. In the SR view (`v`) the
//! disk box shows the SR table instead.

use super::sr::sr_table;
use super::widgets::*;
use super::{bold, boxed, dim, lat_color};
use crate::app::App;
use crate::fmt;
use crate::model::Rates;
use crate::theme::{Gradient, Theme};
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::Style;
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

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

pub(super) fn net_box(buf: &mut Buffer, app: &App, r: &Rates, area: Rect) {
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

pub(super) fn disk_box(buf: &mut Buffer, app: &App, r: &Rates, area: Rect) {
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
