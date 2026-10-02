//! Popups: help (`?`), data sources (`i`) and the column chooser (`o`).

use super::columns;
use super::widgets::*;
use super::{bold, dim};
use crate::app::App;
use crate::fmt;
use crate::theme::Theme;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType, Clear, Widget};

/// Every key, by topic, for `?`. Keep README's key table in step.
pub const HELP: &[(&str, &[(&str, &str)])] = &[
    (
        "navigate",
        &[
            ("↑ ↓  j k  wheel", "select domain"),
            ("PgUp PgDn  g G", "page / first / last"),
            ("⏎  space  dbl-click", "toggle domain details"),
            ("esc", "close details / clear filter"),
        ],
    ),
    (
        "sort & filter",
        &[
            ("s S  ← →", "next / previous sort column"),
            ("click a title", "sort by it; again: reverse"),
            ("c m n d l", "sort by cpu, mem, net, disk, latency"),
            ("r", "reverse sort order"),
            ("0", "pin Domain-0 on top"),
            ("/  f", "filter by name or id"),
        ],
    ),
    (
        "view",
        &[
            ("1 2 3 4", "toggle cpu / mem / net / disk boxes"),
            ("5", "domains only (again: restore boxes)"),
            ("v", "disk box: graphs / per-SR totals"),
            ("o", "choose and reorder columns"),
            ("t  T", "next / previous colour theme"),
            ("i", "data sources (what libxenstat provides)"),
        ],
    ),
    (
        "sampling & settings",
        &[
            ("+  -", "slower / faster refresh"),
            ("p", "pause sampling"),
            ("W", "save settings now (also saved on quit)"),
            ("?  h  F1", "this help"),
            ("q  ctrl-c", "quit"),
        ],
    ),
];

const HELP_KEY_W: usize = 23;

fn help_group(th: &Theme, title: &str, keys: &[(&str, &str)]) -> Vec<Line<'static>> {
    let mut v = vec![Line::from(Span::styled(
        title.to_string(),
        Style::new().fg(th.title).add_modifier(Modifier::BOLD),
    ))];
    for (k, what) in keys {
        v.push(Line::from(vec![
            Span::styled(format!("  {k:<w$}", w = HELP_KEY_W - 2), Style::new().fg(th.key)),
            Span::styled(what.to_string(), Style::new().fg(th.fg)),
        ]));
    }
    v.push(Line::from(""));
    v
}

/// Help: grouped by topic, in two columns when the screen is wide enough,
/// scrollable (↑↓) when it still doesn't fit.
pub(super) fn help_popup(buf: &mut Buffer, app: &mut App, area: Rect) {
    let th = app.theme();
    let groups: Vec<Vec<Line>> = HELP.iter().map(|(t, k)| help_group(th, t, k)).collect();
    let desc_w = HELP
        .iter()
        .flat_map(|g| g.1.iter())
        .map(|(_, w)| fmt::width(w))
        .max()
        .unwrap_or(0);
    let col_w = HELP_KEY_W + desc_w;
    let total: usize = groups.iter().map(|g| g.len()).sum();
    // Two columns: split the groups where the taller side is shortest.
    let split = (1..groups.len())
        .min_by_key(|&i| {
            let left: usize = groups[..i].iter().map(|g| g.len()).sum();
            left.max(total - left)
        })
        .unwrap_or(groups.len());
    let two = area.width as usize >= 2 * col_w + 8 && groups.len() > 1;
    let (left, right): (Vec<Line>, Vec<Line>) = if two {
        (groups[..split].concat(), groups[split..].concat())
    } else {
        (groups.concat(), Vec::new())
    };
    let mut rows = left.len().max(right.len());
    // The last group's trailing blank line isn't needed.
    rows = rows.saturating_sub(1);
    let width = if two { 2 * col_w + 7 } else { col_w + 4 };
    let avail = area.height.saturating_sub(4) as usize;
    let max_scroll = rows.saturating_sub(avail);
    app.help_scroll = app.help_scroll.min(max_scroll);
    let scroll = app.help_scroll;
    let footer = match max_scroll {
        0 => " any key to close ",
        m if scroll < m => " ▼ more · ↑↓ scroll · other keys close ",
        _ => " ▲ more · ↑↓ scroll · other keys close ",
    };
    let (outer, inner) = popup_frame(buf, th, area, " help ", width as u16, rows as u16 + 4, footer);
    app.popup_area = outer;
    let body = Rect::new(
        inner.x + 1,
        inner.y + 1,
        inner.width.saturating_sub(2),
        inner.height.saturating_sub(2),
    );
    for (ci, col) in [left, right].iter().enumerate() {
        let x = body.x + (ci * (col_w + 3)) as u16;
        if x >= body.x + body.width {
            break;
        }
        let w = (body.x + body.width - x).min(col_w as u16);
        for (i, l) in col.iter().skip(scroll).take(body.height as usize).enumerate() {
            put(buf, x, body.y + i as u16, w, l);
        }
    }
}

/// Clear a centered `width` x `height` box (clipped to `area`) and draw a
/// popup frame. Returns the outer and inner rectangles.
fn popup_frame(
    buf: &mut Buffer,
    th: &Theme,
    area: Rect,
    title: &'static str,
    width: u16,
    height: u16,
    footer: &'static str,
) -> (Rect, Rect) {
    let w = width.min(area.width);
    let h = height.min(area.height);
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
        .title_bottom(Line::from(dim(th, footer)).right_aligned());
    let inner = block.inner(r);
    block.render(r, buf);
    (r, inner)
}

/// Centered, bordered popup listing `lines`. Returns its area.
fn popup(
    buf: &mut Buffer,
    th: &Theme,
    area: Rect,
    title: &'static str,
    width: u16,
    lines: Vec<Line>,
) -> Rect {
    let (r, inner) = popup_frame(
        buf,
        th,
        area,
        title,
        width,
        lines.len() as u16 + 4,
        " any key to close ",
    );
    for (i, l) in lines.iter().enumerate() {
        let y = inner.y + 1 + i as u16;
        if y >= inner.y + inner.height {
            break;
        }
        put(buf, inner.x + 1, y, inner.width.saturating_sub(2), l);
    }
    r
}

/// Column chooser (`o`): every column with a checkbox, in display order.
pub(super) fn chooser_popup(buf: &mut Buffer, app: &mut App, area: Rect) {
    let th = app.theme();
    let n = app.columns.len();
    let cursor = app.chooser.unwrap_or(0).min(n.saturating_sub(1));
    let (outer, inner) = popup_frame(buf, th, area, " columns ", 76, n as u16 + 4, " esc close ");
    app.popup_area = outer;
    if inner.width < 10 || inner.height < 2 {
        app.chooser_hits = Default::default();
        return;
    }
    let x = inner.x + 1;
    let w = inner.width.saturating_sub(2);
    let k = |s: &'static str| Span::styled(s, Style::new().fg(th.key));
    put(
        buf,
        x,
        inner.y,
        w,
        &Line::from(vec![
            k("space"),
            dim(th, " show/hide  "),
            k("J K"),
            dim(th, " or "),
            k("⇧↑↓"),
            dim(th, " move  "),
            k("d"),
            dim(th, " defaults  "),
            dim(th, "click: toggle, ▲▼ move"),
        ]),
    );
    let rows = Rect::new(x, inner.y + 2, w, inner.height.saturating_sub(2));
    let vis = rows.height as usize;
    let scroll = if vis == 0 {
        0
    } else {
        let s = app.chooser_hits.scroll.min(cursor);
        if cursor >= s + vis {
            cursor + 1 - vis
        } else {
            s
        }
    };
    // "▸ [x] ▲▼ TITLE..."
    let up_x = x + 6;
    app.chooser_hits = crate::app::ChooserHits {
        rows,
        scroll,
        up_x,
        down_x: up_x + 1,
    };
    for (i, (id, on)) in app.columns.iter().enumerate().skip(scroll).take(vis) {
        let Some(c) = columns::column(id) else { continue };
        let y = rows.y + (i - scroll) as u16;
        let check = match (c.locked, on) {
            (true, _) => "[•]",
            (false, true) => "[x]",
            (false, false) => "[ ]",
        };
        let fg = if *on { th.fg } else { th.dim };
        let mut sp = vec![
            Span::styled(if i == cursor { "▸ " } else { "  " }, Style::new().fg(th.key)),
            Span::styled(
                format!("{check} "),
                Style::new().fg(if *on { th.key } else { th.dim }),
            ),
            dim(th, "▲▼ "),
            Span::styled(
                format!("{:<12}", c.title),
                Style::new().fg(fg).add_modifier(Modifier::BOLD),
            ),
            Span::styled(c.about, Style::new().fg(fg)),
        ];
        if c.locked {
            sp.push(dim(th, "  (always)"));
        } else if app.cols_dropped.contains(&c.id) {
            sp.push(Span::styled("  (no room)", Style::new().fg(th.warn)));
        }
        put(buf, x, y, w, &Line::from(sp));
        if i == cursor {
            buf.set_style(Rect::new(x, y, w, 1), Style::new().bg(th.sel_bg));
        }
    }
}

pub(super) fn info_popup(buf: &mut Buffer, app: &mut App, area: Rect) {
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
        {
            use crate::source::XapiState;
            let (mark, c, how) = match &st.xapi {
                XapiState::Connected => ("✓", th.ok, "xapi".to_string()),
                XapiState::Connecting => ("◐", th.warn, "xapi: connecting".into()),
                XapiState::Failed(e) => ("✗", th.bad, format!("xapi: {e}")),
                XapiState::Disabled => ("·", th.dim, "off (--no-xapi): UUIDs only".into()),
                XapiState::Absent => ("·", th.dim, "n/a: no xapi (plain Xen), UUIDs only".into()),
            };
            Line::from(vec![
                Span::styled(format!("{mark} "), Style::new().fg(c)),
                Span::styled(format!("{:<20}", "SR/disk/net names"), Style::new().fg(th.fg)),
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
    app.popup_area = popup(buf, th, area, " data sources ", 70, lines);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Every key the app handles is in the help.
    #[test]
    fn help_lists_every_key() {
        let keys: String = HELP
            .iter()
            .flat_map(|g| g.1.iter())
            .map(|(k, _)| format!(" {k} "))
            .collect();
        for k in [
            "j", "k", "g", "G", "s", "S", "r", "c", "0", "/", "f", "5", "o", "t", "T", "+", "-", "p", "W",
            "?", "h", "F1", "q", "esc", "i",
        ] {
            assert!(keys.contains(&format!(" {k} ")), "{k} missing from help");
        }
    }
}
