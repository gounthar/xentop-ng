//! The domain table's column registry.
//!
//! Every column is one [`Column`] entry in [`COLUMNS`]: its id (also used in
//! the config file and as its sort key), title, width, how early it is
//! dropped when the terminal is narrow, how it sorts and how a cell is
//! drawn. Adding a column means adding one entry here; the chooser (`o`),
//! header clicks, `s`/`S` cycling and the config file pick it up.

use super::{lat_color, steal_color, widgets::*};
use crate::fmt;
use crate::history::DomHistory;
use crate::model::{DomRates, DomState};
use crate::theme::Theme;
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::Span;
use std::cmp::Ordering;

/// What a cell renderer gets besides the domain itself.
pub struct RowCtx<'a> {
    pub th: &'a Theme,
    pub hist: Option<&'a DomHistory>,
}

/// Draws one cell, exactly `width` columns wide.
pub type Render = fn(&RowCtx, &DomRates, usize) -> Vec<Span<'static>>;

/// How a column sorts the table.
pub enum Sort {
    /// Not sortable.
    None,
    /// Numeric, biggest first unless reversed.
    Desc(fn(&DomRates) -> f64),
    /// Numeric, smallest first unless reversed.
    Asc(fn(&DomRates) -> f64),
    /// Case-insensitive text, A to Z unless reversed.
    Text(fn(&DomRates) -> String),
    /// Sorts like another sort key (CPU history sorts by CPU).
    Same(&'static str),
}

pub struct Column {
    /// Stable identifier: config file, sort key.
    pub id: &'static str,
    pub title: &'static str,
    /// One-line description for the column chooser.
    pub about: &'static str,
    pub width: usize,
    /// Lower is kept longer when the terminal is too narrow.
    pub priority: u8,
    pub right: bool,
    pub default_on: bool,
    /// Can't be hidden (ID and NAME: a VM can be *named* "Domain-0").
    pub locked: bool,
    /// Takes the spare width once the name has grown a bit.
    pub flex: bool,
    pub sort: Sort,
    pub render: Render,
}

/// Sort keys that aren't a single column: `n` and `d` sort by total
/// network / disk throughput, and highlight both halves.
pub struct Aggregate {
    pub id: &'static str,
    pub label: &'static str,
    pub sort: Sort,
    pub cols: &'static [&'static str],
}

pub static AGGREGATES: &[Aggregate] = &[
    Aggregate {
        id: "net",
        label: "net",
        sort: Sort::Desc(DomRates::net_bps),
        cols: &["net_rx", "net_tx"],
    },
    Aggregate {
        id: "disk",
        label: "disk",
        sort: Sort::Desc(DomRates::disk_bps),
        cols: &["disk_rd", "disk_wr"],
    },
];

fn pad(s: String, w: usize, right: bool) -> String {
    fmt::pad(&s, w, right)
}

/// A right-aligned number, dimmed when zero.
fn num(th: &Theme, v: String, zero: bool, w: usize, c: Color) -> Vec<Span<'static>> {
    vec![Span::styled(
        pad(v, w, true),
        Style::new().fg(if zero { th.dim } else { c }),
    )]
}

pub static COLUMNS: &[Column] = &[
    Column {
        id: "id",
        title: "ID",
        about: "domain id",
        width: 4,
        priority: 0,
        right: true,
        default_on: true,
        locked: true,
        flex: false,
        sort: Sort::Asc(|d| d.id as f64),
        render: |cx, d, w| {
            vec![Span::styled(
                pad(d.id.to_string(), w, true),
                Style::new().fg(cx.th.dim),
            )]
        },
    },
    Column {
        id: "name",
        title: "NAME",
        about: "domain name",
        width: 14,
        priority: 0,
        right: false,
        default_on: true,
        locked: true,
        flex: false,
        sort: Sort::Text(|d| d.name.to_lowercase()),
        render: render_name,
    },
    Column {
        id: "state",
        title: "STATE",
        about: "running / idle / paused / crashed",
        width: 6,
        priority: 6,
        right: false,
        default_on: true,
        locked: false,
        flex: false,
        sort: Sort::None,
        render: render_state,
    },
    Column {
        id: "vcpu",
        title: "VCPU",
        about: "vCPUs online / max",
        width: 5,
        priority: 7,
        right: true,
        default_on: true,
        locked: false,
        flex: false,
        sort: Sort::Desc(|d| d.vcpus_online as f64),
        render: |cx, d, w| {
            // xenstat reports max vCPUs; show online/max when they differ.
            let max = d.vcpu_pct.len();
            let t = if d.vcpus_online == max {
                max.to_string()
            } else {
                format!("{}/{max}", d.vcpus_online)
            };
            num(cx.th, t, false, w, cx.th.fg)
        },
    },
    Column {
        id: "cpu",
        title: "CPU",
        about: "CPU use (100% = one pCPU), meter vs vCPUs",
        width: 16,
        priority: 0,
        right: false,
        default_on: true,
        locked: false,
        flex: false,
        sort: Sort::Desc(|d| d.cpu_pct),
        render: render_cpu,
    },
    Column {
        id: "cpu_hist",
        title: "CPU HISTORY",
        about: "CPU use over time",
        width: 12,
        priority: 5,
        right: false,
        default_on: true,
        locked: false,
        flex: true,
        sort: Sort::Same("cpu"),
        render: |cx, d, w| {
            let data = cx.hist.map(|h| h.cpu.tail(w)).unwrap_or_default();
            let cap = (d.vcpus_online.max(1) * 100) as f64;
            sparkline(&data, cap, w, &cx.th.cpu, cx.th.meter_empty)
        },
    },
    Column {
        id: "steal",
        title: "STEAL",
        about: "time vCPUs waited for a pCPU (needs hypervisor support)",
        width: 6,
        priority: 4,
        right: true,
        default_on: true,
        locked: false,
        flex: false,
        sort: Sort::Desc(|d| d.steal_pct.unwrap_or(-1.0)),
        render: |cx, d, w| {
            let t = d.steal_pct.map(fmt::pct).unwrap_or_else(|| "-".into());
            vec![Span::styled(
                pad(t, w, true),
                Style::new().fg(steal_color(cx.th, d.steal_pct)),
            )]
        },
    },
    Column {
        id: "mem",
        title: "MEM",
        about: "current memory",
        width: 7,
        priority: 0,
        right: true,
        default_on: true,
        locked: false,
        flex: false,
        sort: Sort::Desc(|d| d.mem as f64),
        render: |cx, d, w| num(cx.th, fmt::bytes(d.mem as f64), false, w, cx.th.mem.at(0.6)),
    },
    Column {
        id: "net_rx",
        title: "NET ▼",
        about: "network traffic to the VM, B/s",
        width: 8,
        priority: 1,
        right: true,
        default_on: true,
        locked: false,
        flex: false,
        sort: Sort::Desc(|d| d.net_rx_bps),
        render: |cx, d, w| {
            let v = d.net_rx_bps;
            num(cx.th, fmt::rate(v), v < 0.5, w, cx.th.rx.at(1.0))
        },
    },
    Column {
        id: "net_tx",
        title: "NET ▲",
        about: "network traffic from the VM, B/s",
        width: 8,
        priority: 1,
        right: true,
        default_on: true,
        locked: false,
        flex: false,
        sort: Sort::Desc(|d| d.net_tx_bps),
        render: |cx, d, w| {
            let v = d.net_tx_bps;
            num(cx.th, fmt::rate(v), v < 0.5, w, cx.th.tx.at(1.0))
        },
    },
    Column {
        id: "disk_rd",
        title: "READ",
        about: "disk reads, B/s",
        width: 8,
        priority: 2,
        right: true,
        default_on: true,
        locked: false,
        flex: false,
        sort: Sort::Desc(|d| d.disk_rd_bps),
        render: |cx, d, w| {
            let v = d.disk_rd_bps;
            num(cx.th, fmt::rate(v), v < 0.5, w, cx.th.rd.at(1.0))
        },
    },
    Column {
        id: "disk_wr",
        title: "WRITE",
        about: "disk writes, B/s",
        width: 8,
        priority: 2,
        right: true,
        default_on: true,
        locked: false,
        flex: false,
        sort: Sort::Desc(|d| d.disk_wr_bps),
        render: |cx, d, w| {
            let v = d.disk_wr_bps;
            num(cx.th, fmt::rate(v), v < 0.5, w, cx.th.wr.at(1.0))
        },
    },
    Column {
        id: "iops",
        title: "IOPS",
        about: "disk requests per second, read + write",
        width: 7,
        priority: 4,
        right: true,
        default_on: true,
        locked: false,
        flex: false,
        sort: Sort::Desc(|d| d.disk_rd_iops + d.disk_wr_iops),
        render: |cx, d, w| {
            let v = d.disk_rd_iops + d.disk_wr_iops;
            num(cx.th, fmt::count(v), v < 0.5, w, cx.th.fg)
        },
    },
    Column {
        id: "lat",
        title: "LAT",
        about: "disk latency, worst of read/write (! = 5 ms+)",
        width: 8,
        priority: 3,
        right: true,
        default_on: true,
        locked: false,
        flex: false,
        sort: Sort::Desc(|d| d.lat_us().unwrap_or(-1.0)),
        render: |cx, d, w| {
            let l = d.lat_us();
            vec![Span::styled(
                pad(lat_text(l), w, true),
                Style::new().fg(lat_color(cx.th, l)),
            )]
        },
    },
];

/// Latency text with a trailing "!" when slow, so the warning doesn't rely
/// on colour alone.
pub fn lat_text(us: Option<f64>) -> String {
    let s = fmt::lat(us);
    if us.is_some_and(|u| u >= SLOW_US) {
        s + "!"
    } else {
        s
    }
}

/// Latency at which a value gets flagged with "!".
pub const SLOW_US: f64 = 5_000.0;

fn render_name(cx: &RowCtx, d: &DomRates, w: usize) -> Vec<Span<'static>> {
    let th = cx.th;
    let st = match d.state {
        _ if d.id == 0 => Style::new().fg(th.title).add_modifier(Modifier::BOLD),
        Some(DomState::Crashed) => Style::new().fg(th.bad).add_modifier(Modifier::BOLD),
        Some(DomState::Paused) | Some(DomState::Shutdown) | Some(DomState::Dying) => {
            Style::new().fg(th.dim).add_modifier(Modifier::ITALIC)
        }
        _ => Style::new().fg(th.fg),
    };
    vec![Span::styled(pad(d.name.clone(), w, false), st)]
}

fn render_state(cx: &RowCtx, d: &DomRates, w: usize) -> Vec<Span<'static>> {
    let th = cx.th;
    // Xen's running/blocked flags are an instantaneous snapshot (a busy VM
    // is usually "blocked" at any given moment), so judge activity by CPU
    // use over the interval instead.
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
    vec![Span::styled(pad(t.into(), w, false), Style::new().fg(c))]
}

fn render_cpu(cx: &RowCtx, d: &DomRates, w: usize) -> Vec<Span<'static>> {
    let th = cx.th;
    let cap = (d.vcpus_online.max(1) * 100) as f64;
    let f = d.cpu_pct / cap;
    let mw = w.saturating_sub(7);
    let mut sp = meter(f, mw, &th.cpu, th.meter_empty);
    sp.push(Span::styled(
        format!("{:>7}", fmt::pct(d.cpu_pct)),
        Style::new().fg(if d.cpu_pct < 0.5 {
            th.dim
        } else {
            th.cpu.at(f.max(0.15))
        }),
    ));
    sp
}

// ---------------------------------------------------------------------------
// Lookups

pub fn column(id: &str) -> Option<&'static Column> {
    COLUMNS.iter().find(|c| c.id == id)
}

/// The sort key a column sorts by (its own id, or the one it borrows), if
/// it is sortable.
pub fn sort_of(c: &Column) -> Option<&'static str> {
    match c.sort {
        Sort::None => None,
        Sort::Same(id) => Some(id),
        _ => Some(c.id),
    }
}

fn sort_def(id: &str) -> Option<&'static Sort> {
    if let Some(a) = AGGREGATES.iter().find(|a| a.id == id) {
        return Some(&a.sort);
    }
    match column(id).map(|c| &c.sort) {
        Some(Sort::None) | None => None,
        Some(Sort::Same(other)) => sort_def(other),
        Some(s) => Some(s),
    }
}

/// Is `id` something the table can be sorted by?
pub fn is_sort_key(id: &str) -> bool {
    sort_def(id).is_some()
}

/// Turn a sort key from a config file into the registry's own `&'static`.
pub fn sort_key(id: &str) -> Option<&'static str> {
    let s = AGGREGATES
        .iter()
        .map(|a| a.id)
        .chain(COLUMNS.iter().filter_map(sort_of))
        .find(|k| *k == id)?;
    is_sort_key(s).then_some(s)
}

/// Short name of a sort key for the table title.
pub fn sort_label(id: &str) -> String {
    if let Some(a) = AGGREGATES.iter().find(|a| a.id == id) {
        return a.label.to_string();
    }
    column(id)
        .map(|c| c.title.to_lowercase())
        .unwrap_or_else(|| id.to_string())
}

/// Should column `col` show the sort arrow for sort key `sort`?
pub fn shows_sort(col: &Column, sort: &str) -> bool {
    col.id == sort
        || AGGREGATES
            .iter()
            .any(|a| a.id == sort && a.cols.contains(&col.id))
}

/// Compare two domains by sort key `id` in its natural direction (numbers
/// biggest first, names and ids ascending). Unknown keys compare equal.
pub fn compare(id: &str, a: &DomRates, b: &DomRates) -> Ordering {
    match sort_def(id) {
        Some(Sort::Desc(f)) => f(b).total_cmp(&f(a)),
        Some(Sort::Asc(f)) => f(a).total_cmp(&f(b)),
        Some(Sort::Text(f)) => f(a).cmp(&f(b)),
        _ => Ordering::Equal,
    }
}

/// Default column set, in registry order.
pub fn defaults() -> Vec<(&'static str, bool)> {
    COLUMNS.iter().map(|c| (c.id, c.default_on)).collect()
}

/// Lay out the enabled columns (in the user's order) for `width` cells.
/// Columns are dropped by priority until the rest fit; `name_w` (the
/// longest name) only decides how much of the spare width the name takes.
/// Returns the columns with their widths, and the ids of enabled columns
/// that didn't fit.
pub fn layout(
    enabled: &[&'static Column],
    width: u16,
    name_w: usize,
) -> (Vec<(&'static Column, usize)>, Vec<&'static str>) {
    let width = width as usize;
    let mut max_prio = enabled.iter().map(|c| c.priority).max().unwrap_or(0);
    loop {
        let mut cols: Vec<(&'static Column, usize)> = enabled
            .iter()
            .filter(|c| c.priority <= max_prio)
            .map(|c| (*c, c.width))
            .collect();
        let used: usize = cols.iter().map(|c| c.1 + 1).sum();
        if used <= width || max_prio == 0 {
            let mut extra = width.saturating_sub(used);
            // Grow the name a bit, or to the longest name (up to half the
            // spare width next to a flexible column), then give everything
            // else to the flexible column (the history). Without one, the
            // rest stays blank at the end of the row.
            let flex = cols.iter().any(|c| c.0.flex);
            if let Some(c) = cols.iter_mut().find(|c| c.0.id == "name") {
                let need = name_w.saturating_sub(c.1);
                let g = if flex {
                    need.min((extra / 2).max(12)).max(12)
                } else {
                    need.max(26)
                }
                .min(extra);
                c.1 += g;
                extra -= g;
            }
            if let Some(c) = cols.iter_mut().find(|c| c.0.flex) {
                c.1 += extra;
            }
            let dropped = enabled
                .iter()
                .filter(|c| c.priority > max_prio)
                .map(|c| c.id)
                .collect();
            return (cols, dropped);
        }
        max_prio -= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registry_is_consistent() {
        let mut ids: Vec<&str> = COLUMNS.iter().map(|c| c.id).collect();
        ids.extend(AGGREGATES.iter().map(|a| a.id));
        let n = ids.len();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), n, "duplicate column/sort ids");
        for c in COLUMNS {
            assert!(c.width >= 3, "{}", c.id);
            assert!(!c.title.is_empty() && !c.about.is_empty(), "{}", c.id);
            if let Sort::Same(other) = c.sort {
                assert!(is_sort_key(other), "{} borrows unknown sort {other}", c.id);
            }
            // Every column draws exactly its width.
            let th = &crate::theme::THEMES[0];
            let d = DomRates {
                name: "a-rather-long-domain-name".into(),
                vcpu_pct: vec![10.0; 4],
                vcpus_online: 4,
                ..Default::default()
            };
            for w in [c.width, c.width + 7] {
                let sp = (c.render)(&RowCtx { th, hist: None }, &d, w);
                let got: usize = sp.iter().map(|s| fmt::width(&s.content)).sum();
                assert_eq!(got, w, "column {} at width {w}", c.id);
            }
        }
        for a in AGGREGATES {
            for c in a.cols {
                assert!(column(c).is_some(), "{} highlights unknown {c}", a.id);
            }
        }
    }

    #[test]
    fn every_numeric_column_sorts() {
        for id in [
            "iops", "vcpu", "mem", "net_rx", "net_tx", "disk_rd", "disk_wr", "lat", "cpu",
        ] {
            assert!(is_sort_key(id), "{id}");
            assert_eq!(sort_key(id), Some(id));
        }
        assert_eq!(sort_key("cpu_hist"), None, "history sorts as cpu, not by itself");
        assert_eq!(sort_of(column("cpu_hist").unwrap()), Some("cpu"));
        assert!(!is_sort_key("state"));
        assert_eq!(sort_key("bogus"), None);
        assert_eq!(sort_label("net"), "net");
        assert_eq!(sort_label("net_rx"), "net ▼");
        assert!(shows_sort(column("net_tx").unwrap(), "net"));
        assert!(!shows_sort(column("mem").unwrap(), "net"));
    }

    #[test]
    fn compare_directions() {
        let a = DomRates {
            id: 1,
            name: "beta".into(),
            disk_rd_iops: 5.0,
            ..Default::default()
        };
        let b = DomRates {
            id: 2,
            name: "Alpha".into(),
            disk_rd_iops: 50.0,
            disk_rd_lat_us: Some(100.0),
            ..Default::default()
        };
        assert_eq!(compare("iops", &a, &b), Ordering::Greater, "biggest first");
        assert_eq!(compare("id", &a, &b), Ordering::Less, "ids ascending");
        assert_eq!(compare("name", &a, &b), Ordering::Greater, "case-insensitive");
        assert_eq!(compare("lat", &a, &b), Ordering::Greater, "missing latency last");
        assert_eq!(compare("cpu_hist", &a, &b), Ordering::Equal);
        assert_eq!(compare("nope", &a, &b), Ordering::Equal);
    }

    #[test]
    fn layout_drops_by_priority_and_reports_it() {
        let all: Vec<&Column> = COLUMNS.iter().collect();
        let (cols, dropped) = layout(&all, 300, 8);
        assert_eq!(cols.len(), COLUMNS.len());
        assert!(dropped.is_empty());
        let total: usize = cols.iter().map(|c| c.1 + 1).sum();
        assert_eq!(total, 300, "spare width is handed out");

        let (cols, dropped) = layout(&all, 80, 8);
        let used: usize = cols.iter().map(|c| c.1 + 1).sum();
        assert!(used <= 80);
        assert!(!dropped.is_empty());
        assert!(dropped.contains(&"vcpu"), "lowest priority goes first");
        for keep in ["id", "name", "cpu", "mem"] {
            assert!(cols.iter().any(|c| c.0.id == keep), "{keep}");
        }
        // User order is preserved.
        let mut mine: Vec<&Column> = ["mem", "name", "id"].iter().map(|i| column(i).unwrap()).collect();
        mine.push(column("lat").unwrap());
        let (cols, _) = layout(&mine, 200, 8);
        let ids: Vec<&str> = cols.iter().map(|c| c.0.id).collect();
        assert_eq!(ids, ["mem", "name", "id", "lat"]);
        // Absurdly narrow: priority-0 columns stay, nothing panics.
        let (cols, _) = layout(&all, 3, 8);
        assert!(cols.iter().all(|c| c.0.priority == 0));
    }

    #[test]
    fn layout_grows_the_name_to_fit() {
        let all: Vec<&Column> = COLUMNS.iter().collect();
        let name = |cols: &[(&Column, usize)]| cols.iter().find(|c| c.0.id == "name").unwrap().1;
        let (short, _) = layout(&all, 300, 8);
        assert_eq!(name(&short), 14 + 12, "short names: as before");
        let (long, _) = layout(&all, 300, 40);
        assert_eq!(name(&long), 40, "a long name fits when there is room");
        let hist = long.iter().find(|c| c.0.flex).unwrap().1;
        assert!(hist >= 40, "the history keeps half the spare width");
        // Long names never push a column out.
        let (a, da) = layout(&all, 120, 8);
        let (b, db) = layout(&all, 120, 64);
        assert_eq!((a.len(), da), (b.len(), db));
        let used: usize = b.iter().map(|c| c.1 + 1).sum();
        assert!(used <= 120);
    }
}
