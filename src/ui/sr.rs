//! The SR view (`v`): per-SR totals and the busiest disks.

use super::widgets::*;
use super::{dim, lat_color, trunc_left};
use crate::app::App;
use crate::fmt;
use crate::model::{DomRates, Rates};
use crate::theme::Theme;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};

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
    Trend,
    Top,
}

/// SR table columns that fit `width`, the trend and top-VM columns taking
/// what's left.
/// `sr_w` and `kind_w` are what the SR names and types would like.
fn sr_cols(width: usize, sr_w: usize, kind_w: usize) -> Vec<(SrCol, usize)> {
    let sr_full = sr_w.max(8);
    let sr_w = sr_w.clamp(8, 20);
    // (column, width, priority: lower = kept longer)
    let all = [
        (SrCol::Sr, sr_w, 0),
        (SrCol::Kind, kind_w.clamp(4, 9), 4),
        (SrCol::Vbds, 4, 5),
        (SrCol::Iops, 6, 0),
        (SrCol::Read, 7, 3),
        (SrCol::Write, 7, 3),
        (SrCol::RLat, 7, 2),
        (SrCol::WLat, 7, 0),
        (SrCol::Trend, 10, 1),
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
            let mut spare = width.saturating_sub(used);
            // VM names readable first, then a longer trend, then the rest,
            // then SR names past 20 columns.
            let more = [
                (SrCol::Top, 8),
                (SrCol::Trend, 20),
                (SrCol::Top, 16),
                (SrCol::Sr, sr_full - sr_w),
            ];
            for (col, max) in more {
                if let Some(c) = cols.iter_mut().find(|c| c.0 == col) {
                    let add = spare.min(max);
                    c.1 += add;
                    spare -= add;
                }
            }
            // Still too wide: long SR names give way first.
            if let Some(c) = cols.iter_mut().find(|c| c.0 == SrCol::Sr) {
                c.1 = c.1.saturating_sub(used.saturating_sub(width)).max(8);
            }
            return cols;
        }
    }
    Vec::new()
}

/// An SR by its name when xapi gave one, else by `short_sr`.
fn sr_label(name: Option<&str>, key: &str, w: usize) -> String {
    match name {
        Some(n) => fmt::trunc(n, w),
        None => short_sr(key, w),
    }
}

/// SR UUIDs are shown by their first block, like `xe` users abbreviate
/// them; other storage keys (directories) by their tail.
pub(super) fn short_sr(key: &str, w: usize) -> String {
    if crate::source::xenstore::uuid(key).is_some() {
        return key.chars().take(8.min(w)).collect();
    }
    trunc_left(key, w)
}

/// Per-SR totals in place of the disk graphs: which SR is slow, and who
/// is hammering it.
pub(super) fn sr_table(buf: &mut Buffer, app: &App, r: &Rates, area: Rect) {
    let srs = &r.srs;
    let th = app.theme();
    if area.height == 0 {
        return;
    }
    if srs.is_empty() {
        // No rows: either no VM disk is active (and no xapi to list the
        // host's SRs anyway), or the disks can't be mapped to storage.
        let msg = if app.status.storage == crate::source::Avail::Missing {
            "no storage mapping (xenstore); see i   v: graphs"
        } else {
            "no VM disks active on this host   v: graphs"
        };
        put(buf, area.x, area.y, area.width, &Line::from(dim(th, msg)));
        return;
    }
    let sr_w = srs
        .iter()
        .map(|s| s.name.as_deref().map_or(8, fmt::width))
        .max()
        .unwrap_or(8);
    let kind_w = srs
        .iter()
        .filter_map(|s| s.kind.as_deref())
        .map(fmt::width)
        .max()
        .unwrap_or(4);
    let cols = sr_cols(area.width as usize, sr_w, kind_w);
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
            SrCol::Trend => ("IOPS TREND", false),
            SrCol::Top => ("TOP VM", false),
        };
        let style = Style::new().fg(th.dim).add_modifier(Modifier::BOLD);
        buf.set_stringn(x, area.y, fmt::pad(t, *w, right), *w, style);
        x += *w as u16 + 1;
    }
    let rows = area.height as usize - 1;
    let hist = &app.hist;
    for (i, s) in in_order(srs, &hist.sr_order, |s| s.sr.clone())
        .take(rows)
        .enumerate()
    {
        let mut sp: Vec<Span> = Vec::new();
        for (c, w) in &cols {
            let w = *w;
            let num = |v: String, c: Color| Span::styled(fmt::pad(&v, w, true), Style::new().fg(c));
            match c {
                SrCol::Sr => sp.push(Span::styled(
                    fmt::pad(&sr_label(s.name.as_deref(), &s.sr, w), w, false),
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
                SrCol::Trend => sp.extend(io_trend(th, hist.srs.get(&s.sr), w)),
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
    // Ranked and filtered on smoothed IOPS, so disks neither trade places
    // nor blink in and out on every sample.
    let vbds: Vec<(&DomRates, &crate::model::VbdRates)> = r
        .domains
        .iter()
        .flat_map(|d| d.vbds.iter().map(move |v| (d, v)))
        .filter(|(d, v)| {
            v.backing.group().is_some() && hist.vbds.get(&(d.id, v.dev)).is_some_and(|h| h.smooth >= 0.5)
        })
        .collect();
    let vbds = in_order(&vbds, &hist.vbd_order, |(d, v)| (d.id, v.dev));
    for &(d, v) in vbds.take((bottom - y) as usize) {
        let mut sp: Vec<Span> = Vec::new();
        for (c, w) in &cols {
            let w = *w;
            let num = |s: String, c: Color| Span::styled(fmt::pad(&s, w, true), Style::new().fg(c));
            match c {
                SrCol::Sr => {
                    let key = v.backing.group().unwrap_or_default();
                    let label = sr_label(v.backing.sr_name.as_deref(), &key, w);
                    sp.push(dim(th, fmt::pad(&label, w, false)));
                }
                SrCol::Kind => sp.push(Span::styled(fmt::pad(&v.name, w, false), Style::new().fg(th.fg))),
                SrCol::Vbds => sp.push(Span::raw(" ".repeat(w))),
                SrCol::Iops => sp.push(num(fmt::count(v.rd_iops + v.wr_iops), th.fg)),
                SrCol::Read => sp.push(num(fmt::rate(v.rd_bps), th.rd.at(1.0))),
                SrCol::Write => sp.push(num(fmt::rate(v.wr_bps), th.wr.at(1.0))),
                SrCol::RLat => sp.push(num(fmt::lat(v.rd_lat_us), lat_color(th, v.rd_lat_us))),
                SrCol::WLat => sp.push(num(fmt::lat(v.wr_lat_us), lat_color(th, v.wr_lat_us))),
                SrCol::Trend => sp.extend(io_trend(th, hist.vbds.get(&(d.id, v.dev)), w)),
                SrCol::Top => sp.extend(vm_cell(th, d.id, &d.name, None, w)),
            }
            sp.push(Span::raw(" "));
        }
        put(buf, area.x, y, area.width, &Line::from(sp));
        y += 1;
    }
}

/// `items` in the order of `order` (by key), then any it doesn't list.
fn in_order<'a, T, K: Eq + std::hash::Hash>(
    items: &'a [T],
    order: &[K],
    key: impl Fn(&T) -> K,
) -> impl Iterator<Item = &'a T> {
    let mut by_key: std::collections::HashMap<K, &T> = items.iter().map(|t| (key(t), t)).collect();
    let mut out: Vec<&T> = order.iter().filter_map(|k| by_key.remove(k)).collect();
    // Not ranked yet: keep their own order.
    out.extend(items.iter().filter(|t| by_key.contains_key(&key(t))));
    out.into_iter()
}

/// IOPS over the last `w` samples, scaled to its own peak; each bar is
/// coloured by the latency at that moment, so a slow spike stands out.
fn io_trend(th: &Theme, h: Option<&crate::history::IoHistory>, w: usize) -> Vec<Span<'static>> {
    const B: [char; 9] = [' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let empty = || Span::styled("▁", Style::new().fg(th.meter_empty));
    let Some(h) = h else {
        return (0..w).map(|_| empty()).collect();
    };
    let n = h.iops.len().min(w);
    let skip = h.iops.len() - n;
    let max = h.iops.iter().skip(skip).copied().fold(0.0, f64::max);
    let mut sp: Vec<Span<'static>> = (n..w).map(|_| empty()).collect();
    for (v, lat) in h.iops.iter().zip(&h.lat).skip(skip) {
        let l = if max >= 0.5 {
            (v / max * 8.0).round() as usize
        } else {
            0
        };
        if l == 0 {
            sp.push(empty());
        } else {
            // No latency counters (blkback, qdisk): plain bars.
            let c = if lat.is_some() { lat_color(th, *lat) } else { th.fg };
            sp.push(Span::styled(B[l.min(8)].to_string(), Style::new().fg(c)));
        }
    }
    sp
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

#[cfg(test)]
mod tests {
    #[test]
    fn sr_names_take_the_leftover_width() {
        use super::{sr_cols, SrCol};
        let sr = |w| {
            sr_cols(w, 30, 4)
                .into_iter()
                .find(|c| c.0 == SrCol::Sr)
                .unwrap()
                .1
        };
        assert_eq!(sr(300), 30, "in full when there is room");
        assert_eq!(sr(110), 20, "spare goes to TOP VM and the trend first");
        assert!(sr(60) >= 8);
    }
}
