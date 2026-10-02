//! The header line: host, Xen version, data source and gaps, toasts,
//! refresh interval and clock.

use super::{bold, dim};
use crate::app::App;
use crate::fmt;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};

fn utc_clock() -> String {
    let s = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
        % 86400;
    format!("{:02}:{:02}:{:02} UTC", s / 3600, s / 60 % 60, s % 60)
}

pub(super) fn header(buf: &mut Buffer, app: &App, area: Rect) {
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
    if let Some(t) = app.current_toast() {
        let max = (area.width as usize / 2).max(10);
        let st = if t.warn {
            Style::new().fg(th.warn).add_modifier(Modifier::BOLD)
        } else {
            Style::new().fg(th.ok)
        };
        right.push(Span::styled(format!(" {} ", fmt::trunc(&t.msg, max)), st));
        right.push(Span::raw(" "));
    }
    if let Some(e) = &app.error {
        right.push(Span::styled(
            format!(" {} ", fmt::trunc(e, 50)),
            Style::new().fg(th.bad),
        ));
    }
    if !app.paused && app.sample_age() > (app.interval * 2).max(std::time::Duration::from_secs(2)) {
        right.push(Span::styled(
            format!(" stale {:.0}s ", app.sample_age().as_secs_f64()),
            Style::new().fg(th.warn),
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
