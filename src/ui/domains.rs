//! The domain list and its footer.

use super::columns;
use super::widgets::*;
use super::{boxed, dim, longest_name};
use crate::app::App;
use crate::fmt;
use crate::model::{DomRates, Rates};
use crate::theme::Theme;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;
use std::time::Instant;

fn pad(s: String, w: usize, right: bool) -> String {
    fmt::pad(&s, w, right)
}

/// Footer hints, most important first; as many as fit are shown, in this
/// order, followed by "? help".
const HINTS: &[(&str, &str)] = &[
    ("⏎", "details"),
    ("s", "sort"),
    ("/", "filter"),
    ("o", "columns"),
    ("↑↓", "select"),
    ("1-5", "boxes"),
    ("v", "SRs"),
    ("t", "theme"),
    ("r", "reverse"),
    ("+-", "speed"),
    ("p", "pause"),
    ("q", "quit"),
];

/// The footer hint line for `width` cells: the most important hints that
/// fit, always ending with "? help".
fn footer_hints(th: &Theme, width: usize) -> Line<'static> {
    let item_w = |k: &str, w: &str| fmt::width(k) + 1 + fmt::width(w) + 2;
    let mut budget = width.saturating_sub(4 + item_w("?", "help"));
    let mut keep = [false; HINTS.len()];
    for (i, (k, w)) in HINTS.iter().enumerate() {
        let need = item_w(k, w);
        if need <= budget {
            keep[i] = true;
            budget -= need;
        }
    }
    let mut v = vec![Span::raw(" ")];
    for (i, (k, w)) in HINTS.iter().enumerate() {
        if keep[i] {
            v.push(Span::styled(*k, Style::new().fg(th.key)));
            v.push(dim(th, format!(" {w}  ")));
        }
    }
    v.push(Span::styled("?", Style::new().fg(th.key)));
    v.push(dim(th, " help "));
    Line::from(v)
}

pub(super) fn domains_box(buf: &mut Buffer, app: &mut App, r: &Rates, area: Rect) {
    let th = app.theme();
    let vis_ids: Vec<u32> = app.visible().iter().map(|d| d.id).collect();
    let arrow = if app.reverse { "▲" } else { "▼" };
    let inner_w = area.width.saturating_sub(2);
    // Every domain, filtered out or not: typing a filter doesn't move the
    // columns.
    let name_w = app
        .frame
        .name_w
        .table
        .hold(longest_name(r.domains.iter()), Instant::now());
    let (cols, dropped) = columns::layout(&app.enabled_columns(), inner_w, name_w);
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
        Span::styled(
            format!("{} {arrow}", columns::sort_label(app.sort)),
            Style::new().fg(th.key),
        ),
    ];
    if !dropped.is_empty() {
        right.push(dim(th, "  "));
        right.push(Span::styled(
            format!("+{} hidden", dropped.len()),
            Style::new().fg(th.warn),
        ));
        right.push(dim(th, " (o)"));
    }
    if !app.filter.is_empty() && !app.filter_edit {
        right.push(dim(th, "  filter "));
        right.push(Span::styled(
            format!("\"{}\"", app.filter),
            Style::new().fg(th.warn),
        ));
    }
    let mut block = boxed(th, "⁵", "domains", right);
    let hints = if app.filter_edit {
        Line::from(vec![
            Span::styled(" filter: ", Style::new().fg(th.key)),
            Span::styled(format!("{}█ ", app.filter), Style::new().fg(th.fg)),
            dim(th, "⏎ apply  esc clear "),
        ])
    } else {
        footer_hints(th, area.width as usize)
    };
    block = block.title_bottom(hints);
    let inner = block.inner(area);
    block.render(area, buf);
    app.frame.cols_dropped = dropped;
    app.frame.head_cells.clear();
    app.frame.table_head = Rect::default();
    if inner.height < 2 {
        app.frame.table_rows = Rect::default();
        return;
    }

    // Header: click a title to sort by it.
    let mut x = inner.x;
    for (c, w) in &cols {
        let active = columns::shows_sort(c, app.sort);
        let t = if active {
            format!("{}{arrow}", c.title)
        } else {
            c.title.to_string()
        };
        let style = if active {
            Style::new()
                .fg(th.key)
                .add_modifier(Modifier::BOLD | Modifier::UNDERLINED)
        } else {
            Style::new().fg(th.dim).add_modifier(Modifier::BOLD)
        };
        let shown = (*w as u16).min((inner.x + inner.width).saturating_sub(x));
        buf.set_stringn(x, inner.y, pad(t, *w, c.right), shown as usize, style);
        app.frame.head_cells.push((x, shown, c.id));
        x = x.saturating_add(*w as u16 + 1);
    }
    app.frame.table_head = Rect::new(inner.x, inner.y, inner.width, 1);

    let rows = Rect::new(inner.x, inner.y + 1, inner.width, inner.height - 1);
    let n_rows = rows.height as usize;
    let sel_idx = app.selected.and_then(|id| vis_ids.iter().position(|&x| x == id));
    let mut off = app.frame.table_offset.min(vis_ids.len().saturating_sub(n_rows));
    if let Some(s) = sel_idx {
        if s < off {
            off = s;
        } else if s >= off + n_rows {
            off = s + 1 - n_rows;
        }
    }
    app.frame.table_offset = off;
    app.frame.table_rows = rows;

    let by_id: std::collections::HashMap<u32, &DomRates> = r.domains.iter().map(|d| (d.id, d)).collect();
    for (i, id) in vis_ids.iter().skip(off).take(n_rows).enumerate() {
        let Some(d) = by_id.get(id) else { continue };
        let y = rows.y + i as u16;
        let cx = columns::RowCtx {
            th,
            hist: app.hist.doms.get(&d.id),
        };
        let mut sp: Vec<Span<'static>> = Vec::new();
        for (c, w) in &cols {
            sp.extend((c.render)(&cx, d, *w));
            sp.push(Span::raw(" "));
        }
        put(buf, rows.x, y, rows.width, &Line::from(sp));
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

#[cfg(test)]
mod tests {
    use super::*;

    /// The footer shows what fits, and always ends with "? help".
    #[test]
    fn footer_adapts_to_width() {
        let th = &crate::theme::THEMES[0];
        let mut prev = 0;
        for w in [10usize, 30, 60, 100, 200] {
            let l = footer_hints(th, w);
            let text: String = l.spans.iter().map(|s| s.content.as_ref()).collect();
            assert!(text.trim_end().ends_with("? help"), "{w}: {text}");
            assert!(w < 20 || l.width() <= w - 2, "{w}: {text}");
            assert!(l.width() >= prev);
            prev = l.width();
        }
        let wide: String = footer_hints(th, 200)
            .spans
            .iter()
            .map(|s| s.content.as_ref())
            .collect();
        for k in ["details", "sort", "filter", "columns", "quit"] {
            assert!(wide.contains(k), "{k}");
        }
    }
}
