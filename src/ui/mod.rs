pub mod columns;
mod cpu;
mod detail;
mod domains;
mod header;
mod mem;
mod netdisk;
mod popups;
mod sr;
mod widgets;

use crate::app::App;
use crate::fmt;
use crate::model::DomRates;
use crate::theme::Theme;
use cpu::cpu_box;
use detail::{detail_box, detail_height};
use domains::domains_box;
use header::header;
use mem::mem_box;
use netdisk::{disk_box, net_box};
use popups::{chooser_popup, help_popup, info_popup};
use ratatui::buffer::Buffer;
use ratatui::layout::{Constraint, Layout, Rect};
use ratatui::style::{Color, Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::{Block, BorderType};
use ratatui::Frame;
use std::cell::Cell;
use std::time::{Duration, Instant};

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
        if th.mono {
            monochrome(buf, th, area);
        }
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
    // Details stacked under the list need the room more than the graph
    // boxes. Drop the net/disk row, unless the SR view (`v`) is on: then
    // the disk box is what was asked for, and the cpu/mem row goes instead.
    let stacked_detail = app.detail && app.selected.is_some() && area.width < 160;
    if stacked_detail && h < 45 {
        if app.sr_view && mid_h > 0 {
            top_h = 0;
        } else {
            mid_h = 0;
        }
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
        help_popup(buf, app, area);
    }
    if app.info {
        info_popup(buf, app, area);
    }
    if app.chooser.is_some() {
        chooser_popup(buf, app, area);
    }
    if th.mono {
        monochrome(buf, th, area);
    } else if app.ansi256 {
        for pos in area.positions() {
            if let Some(c) = buf.cell_mut(pos) {
                c.fg = to_256(c.fg);
                c.bg = to_256(c.bg);
            }
        }
    }
}

/// NO_COLOR: turn the mono theme's role colours into attributes and drop
/// every colour. Highlighted backgrounds (selection, badges) become reverse
/// video, dim text stays dim, keys and warnings bold; empty meter cells get
/// a different glyph so meters still read without colour.
fn monochrome(buf: &mut Buffer, th: &Theme, area: Rect) {
    for pos in area.positions() {
        let Some(c) = buf.cell_mut(pos) else { continue };
        let mut m = c.modifier;
        if c.bg != th.bg && c.bg != Color::Reset {
            m |= Modifier::REVERSED;
        }
        if c.fg == th.meter_empty && c.symbol() == "■" {
            c.set_symbol("·");
        } else if c.fg == th.dim {
            m |= Modifier::DIM;
        } else if c.fg == th.key || c.fg == th.warn {
            m |= Modifier::BOLD;
        }
        c.modifier = m;
        c.fg = Color::Reset;
        c.bg = Color::Reset;
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

// ---------------------------------------------------------------------------
// Name widths (cpu box, mem box, domain list)

/// How long a name column keeps its width after the name that needed it
/// is gone: short-lived VMs (CI jobs, backups) would otherwise shift the
/// columns every time one starts or stops.
const NAME_HOLD: Duration = Duration::from_secs(30);

/// A width that grows at once and shrinks only once nothing has needed it
/// for [`NAME_HOLD`].
#[derive(Default)]
pub struct StickyWidth(Cell<(usize, Option<Instant>)>);

impl StickyWidth {
    pub fn hold(&self, want: usize, now: Instant) -> usize {
        let (w, seen) = self.0.get();
        let keep = want < w && seen.is_some_and(|t| now.duration_since(t) < NAME_HOLD);
        if keep {
            return w;
        }
        self.0.set((want, Some(now)));
        want
    }
}

/// Name widths kept across frames, one per place names are listed.
#[derive(Default)]
pub struct NameWidths {
    pub table: StickyWidth,
    pub cpu: StickyWidth,
    pub mem: StickyWidth,
}

/// Width of a name next to a meter: as long as the longest name, up to
/// half of `room` (what the name and meter share), never capped below
/// `floor`.
fn name_width(longest: usize, room: usize, floor: usize) -> usize {
    longest.min((room / 2).max(floor))
}

fn longest_name<'a>(doms: impl IntoIterator<Item = &'a DomRates>) -> usize {
    doms.into_iter().map(|d| fmt::width(&d.name)).max().unwrap_or(0)
}

// ---------------------------------------------------------------------------
// Shared by several panels

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
pub(super) fn steal_color(th: &Theme, pct: Option<f64>) -> Color {
    match pct {
        Some(p) if p >= 0.5 => th.cpu.at((p / 20.0).max(0.15)),
        _ => th.dim,
    }
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

#[cfg(test)]
mod tests {
    use crate::app::App;
    use crate::source::demo::{DemoConfig, DemoSource};
    use crate::source::Source;
    use ratatui::backend::TestBackend;
    use ratatui::crossterm::event::{KeyCode, KeyEvent};
    use ratatui::Terminal;
    use std::time::{Duration, Instant};

    #[test]
    fn sticky_width_grows_now_and_shrinks_late() {
        let w = super::StickyWidth::default();
        let t0 = Instant::now();
        assert_eq!(w.hold(10, t0), 10);
        assert_eq!(w.hold(30, t0 + Duration::from_secs(1)), 30, "grows at once");
        assert_eq!(w.hold(10, t0 + Duration::from_secs(20)), 30, "held");
        assert_eq!(w.hold(30, t0 + Duration::from_secs(25)), 30, "needed again");
        assert_eq!(w.hold(10, t0 + Duration::from_secs(50)), 30, "hold restarted");
        assert_eq!(w.hold(10, t0 + Duration::from_secs(56)), 10, "then shrinks");
    }

    #[test]
    fn name_width_fits_within_half_the_room() {
        use super::name_width;
        assert_eq!(name_width(40, 200, 12), 40, "fits in full");
        assert_eq!(name_width(40, 60, 12), 30, "half the room");
        assert_eq!(name_width(40, 16, 12), 12, "never below the floor");
        assert_eq!(name_width(5, 200, 12), 5, "short names leave room to the meter");
    }

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
    /// from 1 to 1024 pCPUs, must render without panicking: in the default
    /// theme at every size, and in the colorblind theme and monochrome
    /// (NO_COLOR) at a spread of sizes.
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
        let all = [1u16, 5, 12, 20, 24, 31, 40, 57, 80, 119, 160, 200, 300];
        let some = [1u16, 12, 24, 40, 80, 160, 300];
        let colorblind = crate::theme::by_name("colorblind").unwrap();
        for (pcpus, mem, stock) in hosts {
            let cfg = DemoConfig {
                pcpus,
                mem,
                stock,
                ..Default::default()
            };
            let mut a = app(&cfg);
            for (theme, mono, sizes) in [(0, false, &all[..]), (colorblind, false, &some), (0, true, &some)] {
                a.theme = theme;
                a.mono = mono;
                for &w in sizes {
                    for &h in sizes.iter().filter(|&&h| h <= 80) {
                        let mut term = Terminal::new(TestBackend::new(w, h)).unwrap();
                        for keys in [
                            &[][..],
                            &[KeyCode::Down, KeyCode::Enter],
                            &[KeyCode::Char('?')],
                            &[KeyCode::Char('?'), KeyCode::Down, KeyCode::PageDown],
                            &[KeyCode::Char('i')],
                            &[KeyCode::Char('o')],
                            &[KeyCode::Char('v')],
                            &[KeyCode::Down, KeyCode::Enter],
                            &[KeyCode::Char('v')],
                            &[
                                KeyCode::Char('o'),
                                KeyCode::End,
                                KeyCode::Char('K'),
                                KeyCode::Char(' '),
                            ],
                        ] {
                            for k in keys {
                                a.on_key(KeyEvent::from(*k));
                            }
                            term.draw(|f| super::draw(f, &mut a)).unwrap();
                            a.help = false;
                            a.info = false;
                            a.chooser = None;
                        }
                        a.detail = false;
                        a.sr_view = false;
                    }
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
            super::sr::short_sr("ac70e429-0dec-3ccd-1d24-2713c6104b65", 8),
            "ac70e429"
        );
        assert_eq!(super::trunc_left("/var/lib/xen/images", 8), "…/images");
        assert_eq!(super::trunc_left("/srv", 8), "/srv");
    }

    /// NO_COLOR: not a single colour reaches the terminal, and the
    /// selection is still visible (reverse video).
    #[test]
    fn monochrome_has_no_colour() {
        use ratatui::style::{Color, Modifier};
        let mut a = app(&DemoConfig::default());
        a.mono = true;
        a.on_key(KeyEvent::from(KeyCode::Down));
        a.on_key(KeyEvent::from(KeyCode::Enter));
        let mut term = Terminal::new(TestBackend::new(200, 60)).unwrap();
        term.draw(|f| super::draw(f, &mut a)).unwrap();
        let buf = term.backend().buffer();
        assert!(buf
            .content
            .iter()
            .all(|c| c.fg == Color::Reset && c.bg == Color::Reset));
        let y = a.frame.table_rows.y;
        let reversed = (0..200)
            .filter(|&x| buf[(x, y)].modifier.contains(Modifier::REVERSED))
            .count();
        assert!(reversed > 100, "selected row in reverse video");
    }
}
