use crate::config::{self, ColorMode, Prefs};
use crate::history::History;
use crate::model::{self, DomRates, Rates, Snapshot};
use crate::source::{DataStatus, Source};
use crate::theme::{Theme, MONO, THEMES};
use crate::ui::columns::{self, Column};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Position, Rect};
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Shortcut keys that jump straight to a sort key.
pub const SORT_SHORTCUTS: [(char, &str); 5] = [
    ('c', "cpu"),
    ('m', "mem"),
    ('n', "net"),
    ('d', "disk"),
    ('l', "lat"),
];

/// A short message in the header (config saved, bad config file...).
pub struct Toast {
    pub msg: String,
    pub warn: bool,
    pub until: Instant,
}

/// Where preferences are saved, and what they looked like when loaded, so
/// quitting only writes back what was changed in the UI.
pub struct ConfigState {
    pub path: PathBuf,
    pub file: Prefs,
    pub start: Prefs,
}

/// Mouse targets in the column chooser, filled in while drawing.
#[derive(Default, Clone, Copy)]
pub struct ChooserHits {
    /// The rows listing columns (row i = column `scroll + i`).
    pub rows: Rect,
    pub scroll: usize,
    /// x of the "▲" and "▼" glyphs.
    pub up_x: u16,
    pub down_x: u16,
}

pub struct App {
    pub source: Box<dyn Source>,
    pub source_desc: String,
    prev: Option<Snapshot>,
    pub rates: Option<Rates>,
    pub hist: History,
    pub interval: Duration,
    pub next_sample: Instant,
    pub paused: bool,
    /// Sort key: a column id from the registry, or an aggregate (net, disk).
    pub sort: &'static str,
    pub reverse: bool,
    pub dom0_first: bool,
    pub filter: String,
    pub filter_edit: bool,
    pub selected: Option<u32>,
    pub detail: bool,
    pub help: bool,
    /// First help line shown when the help doesn't fit.
    pub help_scroll: usize,
    pub info: bool,
    /// Column chooser (`o`) open, with the cursor on this column.
    pub chooser: Option<usize>,
    pub chooser_hits: ChooserHits,
    /// Every registry column in display order, and whether it is shown.
    pub columns: Vec<(&'static str, bool)>,
    /// Map colours to the xterm 256-colour palette at the end of each frame.
    pub ansi256: bool,
    /// No colours at all (NO_COLOR, `--colors mono`).
    pub mono: bool,
    /// As configured, so it can be saved back unchanged.
    pub color_mode: ColorMode,
    pub status: DataStatus,
    pub theme: usize,
    pub show: [bool; 4],
    /// Disk box shows per-SR totals instead of throughput graphs (`v`).
    pub sr_view: bool,
    /// Box layout to restore when leaving domains-only mode (`5`).
    saved_show: Option<[bool; 4]>,
    pub error: Option<String>,
    pub toast: Option<Toast>,
    pub config: Option<ConfigState>,
    pub quit: bool,
    /// Where the domain rows were drawn last frame, for mouse hit-testing.
    pub table_rows: Rect,
    pub table_offset: usize,
    /// Table header row and each column's (x, width, id) there.
    pub table_head: Rect,
    pub head_cells: Vec<(u16, u16, &'static str)>,
    /// Columns that are enabled but didn't fit last frame.
    pub cols_dropped: Vec<&'static str>,
    /// Area of the open popup, if any: clicks outside it close it.
    pub popup_area: Rect,
    last_click: Option<(Instant, u32)>,
    /// Name column widths, held across frames.
    pub name_w: crate::ui::NameWidths,
}

impl App {
    pub fn new(source: Box<dyn Source>, interval: Duration, theme: usize) -> Self {
        let source_desc = source.describe();
        App {
            source,
            source_desc,
            prev: None,
            rates: None,
            hist: History::default(),
            interval,
            next_sample: Instant::now(),
            paused: false,
            sort: "cpu",
            reverse: false,
            dom0_first: false,
            filter: String::new(),
            filter_edit: false,
            selected: None,
            detail: false,
            help: false,
            help_scroll: 0,
            info: false,
            chooser: None,
            chooser_hits: ChooserHits::default(),
            columns: columns::defaults(),
            ansi256: false,
            mono: false,
            color_mode: ColorMode::Auto,
            status: DataStatus::default(),
            theme,
            show: [true; 4],
            sr_view: false,
            saved_show: None,
            error: None,
            toast: None,
            config: None,
            quit: false,
            table_rows: Rect::default(),
            table_offset: 0,
            table_head: Rect::default(),
            head_cells: Vec::new(),
            cols_dropped: Vec::new(),
            popup_area: Rect::default(),
            last_click: None,
            name_w: Default::default(),
        }
    }

    pub fn theme(&self) -> &'static Theme {
        if self.mono {
            &MONO
        } else {
            &THEMES[self.theme]
        }
    }

    /// The preferences the config file stores, as they are now.
    pub fn prefs(&self) -> Prefs {
        Prefs {
            theme: self.theme,
            colors: self.color_mode,
            interval: self.interval,
            boxes: self.show,
            sort: self.sort,
            reverse: self.reverse,
            dom0_first: self.dom0_first,
            columns: self.columns.clone(),
        }
    }

    /// Apply preferences. Colour mode needs `$TERM` and whether `$NO_COLOR`
    /// is set.
    pub fn apply_prefs(&mut self, p: &Prefs, term: Option<&str>, no_color: bool) {
        self.theme = p.theme.min(THEMES.len() - 1);
        self.color_mode = p.colors;
        (self.ansi256, self.mono) = p.colors.resolve(term, no_color);
        self.interval = p.interval;
        self.show = p.boxes;
        self.saved_show = None;
        self.sort = p.sort;
        self.reverse = p.reverse;
        self.dom0_first = p.dom0_first;
        self.columns = p.columns.clone();
    }

    pub fn notify(&mut self, msg: impl Into<String>, warn: bool) {
        let secs = if warn { 10 } else { 4 };
        self.toast = Some(Toast {
            msg: msg.into(),
            warn,
            until: Instant::now() + Duration::from_secs(secs),
        });
    }

    /// The toast to show now, if any.
    pub fn current_toast(&self) -> Option<&Toast> {
        self.toast.as_ref().filter(|t| Instant::now() < t.until)
    }

    /// `W`: write every current preference to the config file now.
    pub fn save_config(&mut self) {
        let now = self.prefs();
        let Some(cfg) = &mut self.config else {
            self.notify("no config file (--no-config, or no $HOME)", true);
            return;
        };
        let path = cfg.path.clone();
        match config::save(&path, &now) {
            Ok(()) => {
                cfg.file = now.clone();
                cfg.start = now;
                self.notify(format!("saved {}", path.display()), false);
            }
            Err(e) => self.notify(format!("can't save {}: {e}", path.display()), true),
        }
    }

    /// On quit: write back what was changed in the UI, if anything.
    pub fn save_on_quit(&self) -> Option<std::io::Result<PathBuf>> {
        let cfg = self.config.as_ref()?;
        let now = self.prefs();
        if now == cfg.start {
            return None;
        }
        let out = config::merge(&cfg.file, &cfg.start, &now);
        Some(config::save(&cfg.path, &out).map(|_| cfg.path.clone()))
    }

    /// Take a sample if due. Returns true when the view changed.
    pub fn tick(&mut self) -> bool {
        if Instant::now() < self.next_sample {
            return false;
        }
        self.next_sample = Instant::now() + self.interval;
        if self.paused {
            return false;
        }
        match self.source.sample() {
            Ok(snap) => {
                self.ingest(snap);
                self.status = self.source.status();
                self.error = None;
            }
            Err(e) => self.error = Some(e.to_string()),
        }
        true
    }

    pub fn ingest(&mut self, snap: Snapshot) {
        if let Some(prev) = &self.prev {
            let r = model::compute(prev, &snap);
            self.hist.record(&r);
            self.rates = Some(r);
        }
        self.prev = Some(snap);
    }

    /// Domains after filtering and sorting, in display order.
    pub fn visible(&self) -> Vec<&DomRates> {
        let Some(r) = &self.rates else {
            return Vec::new();
        };
        let f = self.filter.to_lowercase();
        let mut v: Vec<&DomRates> = r
            .domains
            .iter()
            .filter(|d| {
                f.is_empty()
                    || d.name.to_lowercase().contains(&f)
                    || d.id.to_string() == f
                    // VM UUID prefix, as pasted from XO or `xe`.
                    || (f.len() >= 4 && d.vm_uuid.as_deref().is_some_and(|u| u.starts_with(&f)))
            })
            .collect();
        sort_domains(&mut v, self.sort, self.reverse);
        if self.dom0_first {
            if let Some(i) = v.iter().position(|d| d.id == 0) {
                let d0 = v.remove(i);
                v.insert(0, d0);
            }
        }
        v
    }

    /// Columns the user has switched on, in their order.
    pub fn enabled_columns(&self) -> Vec<&'static Column> {
        self.columns
            .iter()
            .filter(|c| c.1)
            .filter_map(|c| columns::column(c.0))
            .collect()
    }

    pub fn selected_index(&self, vis: &[&DomRates]) -> Option<usize> {
        let id = self.selected?;
        vis.iter().position(|d| d.id == id)
    }

    fn move_sel(&mut self, delta: isize) {
        let vis = self.visible();
        if vis.is_empty() {
            return;
        }
        let cur = self.selected_index(&vis);
        let next = match cur {
            None if delta >= 0 => 0,
            None => vis.len() - 1,
            Some(i) => (i as isize + delta).clamp(0, vis.len() as isize - 1) as usize,
        };
        self.selected = Some(vis[next].id);
    }

    /// Sort keys `s`/`S` step through: the sortable columns on screen, in
    /// their order (all enabled ones before the first frame).
    pub fn sort_cycle(&self) -> Vec<&'static str> {
        let on_screen: Vec<&'static Column> = if self.head_cells.is_empty() {
            self.enabled_columns()
        } else {
            self.head_cells
                .iter()
                .filter_map(|c| columns::column(c.2))
                .collect()
        };
        let mut keys: Vec<&'static str> = Vec::new();
        for c in on_screen {
            if let Some(k) = columns::sort_of(c) {
                if !keys.contains(&k) {
                    keys.push(k);
                }
            }
        }
        keys
    }

    fn cycle_sort(&mut self, fwd: bool) {
        let keys = self.sort_cycle();
        if keys.is_empty() {
            return;
        }
        let n = keys.len();
        // From an aggregate (net = rx + tx), start at the column it lights up.
        let cur = keys.iter().position(|k| *k == self.sort).or_else(|| {
            keys.iter()
                .position(|k| columns::column(k).is_some_and(|c| columns::shows_sort(c, self.sort)))
        });
        let next = match cur {
            Some(i) if fwd => (i + 1) % n,
            Some(i) => (i + n - 1) % n,
            None if fwd => 0,
            None => n - 1,
        };
        self.sort = keys[next];
        self.reverse = false;
    }

    /// Sort by `key`; picking the current key again flips the order.
    pub fn sort_by(&mut self, key: &'static str) {
        if self.sort == key {
            self.reverse = !self.reverse;
        } else {
            self.sort = key;
            self.reverse = false;
        }
    }

    pub fn on_key(&mut self, k: KeyEvent) {
        if k.modifiers.contains(KeyModifiers::CONTROL) && k.code == KeyCode::Char('c') {
            self.quit = true;
            return;
        }
        if self.filter_edit {
            match k.code {
                KeyCode::Enter => self.filter_edit = false,
                KeyCode::Esc => {
                    self.filter.clear();
                    self.filter_edit = false;
                }
                KeyCode::Backspace => {
                    self.filter.pop();
                }
                KeyCode::Char(c) => self.filter.push(c),
                _ => {}
            }
            return;
        }
        if self.chooser.is_some() {
            self.chooser_key(k);
            return;
        }
        if self.help {
            match k.code {
                KeyCode::Up | KeyCode::Char('k') => self.help_scroll = self.help_scroll.saturating_sub(1),
                KeyCode::Down | KeyCode::Char('j') => self.help_scroll += 1,
                KeyCode::PageUp => self.help_scroll = self.help_scroll.saturating_sub(10),
                KeyCode::PageDown | KeyCode::Char(' ') => self.help_scroll += 10,
                KeyCode::Home | KeyCode::Char('g') => self.help_scroll = 0,
                _ => self.close_popups(),
            }
            return;
        }
        if self.info {
            self.close_popups();
            return;
        }
        let page = self.table_rows.height.max(1) as isize;
        match k.code {
            KeyCode::Char('q') => self.quit = true,
            KeyCode::Esc => {
                if self.detail {
                    self.detail = false;
                } else if !self.filter.is_empty() {
                    self.filter.clear();
                } else {
                    self.selected = None;
                }
            }
            KeyCode::Up | KeyCode::Char('k') => self.move_sel(-1),
            KeyCode::Down | KeyCode::Char('j') => self.move_sel(1),
            KeyCode::PageUp => self.move_sel(-page),
            KeyCode::PageDown => self.move_sel(page),
            KeyCode::Home | KeyCode::Char('g') => self.move_sel(isize::MIN / 2),
            KeyCode::End | KeyCode::Char('G') => self.move_sel(isize::MAX / 2),
            KeyCode::Enter | KeyCode::Char(' ') => {
                if self.selected.is_none() {
                    self.move_sel(0);
                }
                self.detail = !self.detail;
            }
            KeyCode::Right | KeyCode::Char('>') | KeyCode::Char('s') => self.cycle_sort(true),
            KeyCode::Left | KeyCode::Char('<') | KeyCode::Char('S') => self.cycle_sort(false),
            KeyCode::Char('r') => self.reverse = !self.reverse,
            KeyCode::Char(c) if SORT_SHORTCUTS.iter().any(|s| s.0 == c) => {
                if let Some(&(_, key)) = SORT_SHORTCUTS.iter().find(|s| s.0 == c) {
                    self.sort = key;
                }
            }
            KeyCode::Char('0') => self.dom0_first = !self.dom0_first,
            KeyCode::Char('/') | KeyCode::Char('f') => self.filter_edit = true,
            KeyCode::Char('p') => self.paused = !self.paused,
            KeyCode::Char(c @ ('t' | 'T')) => {
                let n = THEMES.len();
                self.theme = if c == 't' {
                    (self.theme + 1) % n
                } else {
                    (self.theme + n - 1) % n
                };
                if self.mono {
                    self.notify("monochrome (NO_COLOR or --colors mono): themes are off", false);
                } else {
                    self.notify(format!("theme {}", THEMES[self.theme].name), false);
                }
            }
            KeyCode::Char('+') | KeyCode::Char('=') => {
                self.set_interval(self.interval.as_millis() as u64 + 250)
            }
            KeyCode::Char('-') => self.set_interval((self.interval.as_millis() as u64).saturating_sub(250)),
            KeyCode::Char('?') | KeyCode::Char('h') | KeyCode::F(1) => {
                self.help = true;
                self.help_scroll = 0;
            }
            KeyCode::Char('i') => self.info = true,
            KeyCode::Char('o') => self.chooser = Some(0),
            KeyCode::Char('W') => self.save_config(),
            KeyCode::Char(c @ '1'..='4') => {
                let i = c as usize - '1' as usize;
                self.show[i] = !self.show[i];
                self.saved_show = None;
            }
            KeyCode::Char('5') => self.toggle_domains_only(),
            KeyCode::Char('v') => {
                self.sr_view = !self.sr_view;
                // Asking for the SR view means wanting to see it.
                if self.sr_view && !self.show[3] {
                    self.show[3] = true;
                    self.saved_show = None;
                }
            }
            _ => {}
        }
    }

    fn close_popups(&mut self) {
        self.help = false;
        self.info = false;
        self.chooser = None;
        self.popup_area = Rect::default();
    }

    fn chooser_key(&mut self, k: KeyEvent) {
        let Some(cur) = self.chooser else { return };
        let last = self.columns.len().saturating_sub(1);
        let shift = k.modifiers.contains(KeyModifiers::SHIFT);
        match k.code {
            KeyCode::Up if shift => self.move_column(cur, -1),
            KeyCode::Down if shift => self.move_column(cur, 1),
            KeyCode::Char('K') => self.move_column(cur, -1),
            KeyCode::Char('J') => self.move_column(cur, 1),
            KeyCode::Up | KeyCode::Char('k') => self.chooser = Some(cur.saturating_sub(1)),
            KeyCode::Down | KeyCode::Char('j') => self.chooser = Some((cur + 1).min(last)),
            KeyCode::Home | KeyCode::Char('g') => self.chooser = Some(0),
            KeyCode::End | KeyCode::Char('G') => self.chooser = Some(last),
            KeyCode::Char(' ') | KeyCode::Char('x') | KeyCode::Enter => self.toggle_column(cur),
            KeyCode::Char('d') => {
                self.columns = columns::defaults();
                self.notify("default columns", false);
            }
            KeyCode::Esc | KeyCode::Char('o') | KeyCode::Char('q') => self.close_popups(),
            _ => {}
        }
    }

    pub fn toggle_column(&mut self, i: usize) {
        let Some(c) = self.columns.get_mut(i) else { return };
        if columns::column(c.0).is_some_and(|d| d.locked) {
            self.notify("ID and NAME are always shown", false);
            return;
        }
        c.1 = !c.1;
    }

    /// Move column `i` up (`-1`) or down (`1`) in the order, keeping the
    /// chooser's cursor on it.
    pub fn move_column(&mut self, i: usize, delta: isize) {
        let j = i as isize + delta;
        if i >= self.columns.len() || j < 0 || j as usize >= self.columns.len() {
            return;
        }
        self.columns.swap(i, j as usize);
        if self.chooser.is_some() {
            self.chooser = Some(j as usize);
        }
    }

    /// `5`: hide every box but the domain list, or bring back the layout
    /// that was there before.
    pub fn toggle_domains_only(&mut self) {
        if self.show.iter().any(|&s| s) {
            self.saved_show = Some(self.show);
            self.show = [false; 4];
        } else {
            self.show = self.saved_show.take().unwrap_or([true; 4]);
        }
    }

    fn set_interval(&mut self, ms: u64) {
        self.interval = Duration::from_millis(ms.clamp(250, 10_000));
        self.next_sample = Instant::now() + self.interval;
    }

    pub fn on_mouse(&mut self, m: MouseEvent) {
        let pos = Position::new(m.column, m.row);
        if self.help || self.info || self.chooser.is_some() {
            match m.kind {
                MouseEventKind::ScrollUp if self.help => {
                    self.help_scroll = self.help_scroll.saturating_sub(1)
                }
                MouseEventKind::ScrollDown if self.help => self.help_scroll += 1,
                MouseEventKind::ScrollUp => self.chooser_key(KeyEvent::from(KeyCode::Up)),
                MouseEventKind::ScrollDown => self.chooser_key(KeyEvent::from(KeyCode::Down)),
                MouseEventKind::Down(MouseButton::Left) => {
                    if self.chooser.is_some() && self.popup_area.contains(pos) {
                        self.chooser_click(pos);
                    } else {
                        self.close_popups();
                    }
                }
                _ => {}
            }
            return;
        }
        match m.kind {
            MouseEventKind::ScrollUp => self.move_sel(-1),
            MouseEventKind::ScrollDown => self.move_sel(1),
            MouseEventKind::Down(MouseButton::Left) => {
                if self.table_head.contains(pos) {
                    let hit = self
                        .head_cells
                        .iter()
                        .find(|c| m.column >= c.0 && m.column < c.0 + c.1);
                    if let Some(key) = hit.and_then(|c| columns::column(c.2)).and_then(columns::sort_of) {
                        self.sort_by(key);
                    }
                    return;
                }
                let r = self.table_rows;
                if !r.contains(pos) {
                    return;
                }
                let idx = self.table_offset + (m.row - r.y) as usize;
                let vis = self.visible();
                let Some(d) = vis.get(idx) else { return };
                let id = d.id;
                let now = Instant::now();
                if let Some((t, last)) = self.last_click {
                    if last == id && now.duration_since(t) < Duration::from_millis(400) {
                        self.detail = !self.detail;
                    }
                }
                self.last_click = Some((now, id));
                self.selected = Some(id);
            }
            _ => {}
        }
    }

    fn chooser_click(&mut self, pos: Position) {
        let h = self.chooser_hits;
        if !h.rows.contains(pos) {
            return;
        }
        let i = h.scroll + (pos.y - h.rows.y) as usize;
        if i >= self.columns.len() {
            return;
        }
        self.chooser = Some(i);
        if pos.x == h.up_x {
            self.move_column(i, -1);
        } else if pos.x == h.down_x {
            self.move_column(i, 1);
        } else {
            self.toggle_column(i);
        }
    }
}

/// Sort domains by `key` (natural direction, flipped by `reverse`), with a
/// stable tie-break on the id so equal rows don't jitter between frames.
pub fn sort_domains(v: &mut [&DomRates], key: &str, reverse: bool) {
    v.sort_by(|a, b| {
        let o = columns::compare(key, a, b);
        let o = if reverse { o.reverse() } else { o };
        o.then(a.id.cmp(&b.id))
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::source::demo::{DemoConfig, DemoSource};

    fn dom(id: u32, name: &str, iops: f64, mem: u64) -> DomRates {
        DomRates {
            id,
            name: name.into(),
            disk_rd_iops: iops,
            mem,
            ..Default::default()
        }
    }

    fn ids(v: &[&DomRates]) -> Vec<u32> {
        v.iter().map(|d| d.id).collect()
    }

    #[test]
    fn sorts_by_any_key() {
        let ds = [
            dom(0, "Domain-0", 1.0, 4),
            dom(1, "web", 30.0, 8),
            dom(2, "db", 30.0, 2),
        ];
        let mut v: Vec<&DomRates> = ds.iter().collect();
        sort_domains(&mut v, "iops", false);
        assert_eq!(ids(&v), [1, 2, 0], "biggest first, ties by id");
        sort_domains(&mut v, "iops", true);
        assert_eq!(ids(&v), [0, 1, 2], "reversed, ties still by id");
        sort_domains(&mut v, "mem", false);
        assert_eq!(ids(&v), [1, 0, 2]);
        sort_domains(&mut v, "name", false);
        assert_eq!(ids(&v), [2, 0, 1]);
        sort_domains(&mut v, "id", true);
        assert_eq!(ids(&v), [2, 1, 0]);
    }

    fn demo_app() -> App {
        let mut src = DemoSource::new(&DemoConfig::default());
        let hist = src.warmup(5);
        let mut a = App::new(Box::new(src), Duration::from_secs(1), 0);
        for s in hist {
            a.ingest(s);
        }
        a
    }

    #[test]
    fn s_cycles_over_shown_sortable_columns() {
        let mut a = demo_app();
        a.columns = ["id", "name", "state", "cpu", "cpu_hist", "iops"]
            .iter()
            .map(|c| (*c, true))
            .chain([("mem", false)])
            .collect();
        assert_eq!(
            a.sort_cycle(),
            ["id", "name", "cpu", "iops"],
            "no state, cpu once"
        );
        a.sort = "cpu";
        a.reverse = true;
        a.on_key(KeyEvent::from(KeyCode::Char('s')));
        assert_eq!((a.sort, a.reverse), ("iops", false));
        a.on_key(KeyEvent::from(KeyCode::Char('s')));
        assert_eq!(a.sort, "id", "wraps");
        a.on_key(KeyEvent::from(KeyCode::Char('S')));
        assert_eq!(a.sort, "iops");
        // Shortcuts work even for columns that aren't shown.
        a.on_key(KeyEvent::from(KeyCode::Char('m')));
        assert_eq!(a.sort, "mem");
        a.on_key(KeyEvent::from(KeyCode::Char('n')));
        assert_eq!(a.sort, "net");
    }

    #[test]
    fn header_click_sorts_then_reverses() {
        let mut a = demo_app();
        a.table_head = Rect::new(0, 5, 100, 1);
        a.head_cells = vec![
            (0, 4, "id"),
            (5, 14, "name"),
            (20, 12, "cpu_hist"),
            (33, 6, "state"),
        ];
        let click = |a: &mut App, x: u16| {
            a.on_mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: x,
                row: 5,
                modifiers: KeyModifiers::NONE,
            })
        };
        click(&mut a, 7);
        assert_eq!((a.sort, a.reverse), ("name", false));
        click(&mut a, 8);
        assert_eq!((a.sort, a.reverse), ("name", true));
        click(&mut a, 25);
        assert_eq!((a.sort, a.reverse), ("cpu", false), "history sorts as cpu");
        click(&mut a, 34);
        assert_eq!(a.sort, "cpu", "state isn't sortable");
    }

    #[test]
    fn chooser_toggles_and_moves() {
        let mut a = demo_app();
        a.on_key(KeyEvent::from(KeyCode::Char('o')));
        assert_eq!(a.chooser, Some(0));
        a.on_key(KeyEvent::from(KeyCode::Char(' ')));
        assert!(a.columns[0].1, "id is locked");
        a.on_key(KeyEvent::from(KeyCode::Down));
        a.on_key(KeyEvent::from(KeyCode::Down));
        assert_eq!(a.columns[2].0, "state");
        a.on_key(KeyEvent::from(KeyCode::Char(' ')));
        assert!(!a.columns[2].1);
        a.on_key(KeyEvent::from(KeyCode::Char('K')));
        assert_eq!(a.columns[1].0, "state");
        assert_eq!(a.chooser, Some(1), "cursor follows the column");
        a.on_key(KeyEvent::from(KeyCode::Char('q')));
        assert!(
            a.chooser.is_none() && !a.quit,
            "q closes the chooser, doesn't quit"
        );
        assert!(!a.enabled_columns().iter().any(|c| c.id == "state"));
        assert_ne!(a.prefs().columns, columns::defaults());
    }
}
