use crate::history::History;
use crate::model::{self, DomRates, Rates, Snapshot};
use crate::source::Source;
use crate::theme::{Theme, THEMES};
use ratatui::crossterm::event::{KeyCode, KeyEvent, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
use ratatui::layout::{Position, Rect};
use std::time::{Duration, Instant};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SortKey {
    Cpu,
    Mem,
    Net,
    Disk,
    Lat,
    Name,
    Id,
}

impl SortKey {
    pub const ALL: [SortKey; 7] = [
        SortKey::Cpu,
        SortKey::Mem,
        SortKey::Net,
        SortKey::Disk,
        SortKey::Lat,
        SortKey::Name,
        SortKey::Id,
    ];
    pub fn label(self) -> &'static str {
        match self {
            SortKey::Cpu => "cpu",
            SortKey::Mem => "mem",
            SortKey::Net => "net",
            SortKey::Disk => "disk",
            SortKey::Lat => "latency",
            SortKey::Name => "name",
            SortKey::Id => "id",
        }
    }
    /// Natural direction: numbers descending, names/ids ascending.
    fn descending(self) -> bool {
        !matches!(self, SortKey::Name | SortKey::Id)
    }
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
    pub sort: SortKey,
    pub reverse: bool,
    pub dom0_first: bool,
    pub filter: String,
    pub filter_edit: bool,
    pub selected: Option<u32>,
    pub detail: bool,
    pub help: bool,
    pub theme: usize,
    pub show: [bool; 4],
    pub error: Option<String>,
    pub quit: bool,
    /// Where the domain rows were drawn last frame, for mouse hit-testing.
    pub table_rows: Rect,
    pub table_offset: usize,
    last_click: Option<(Instant, u32)>,
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
            sort: SortKey::Cpu,
            reverse: false,
            dom0_first: false,
            filter: String::new(),
            filter_edit: false,
            selected: None,
            detail: false,
            help: false,
            theme,
            show: [true; 4],
            error: None,
            quit: false,
            table_rows: Rect::default(),
            table_offset: 0,
            last_click: None,
        }
    }

    pub fn theme(&self) -> &'static Theme {
        &THEMES[self.theme]
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
            .filter(|d| f.is_empty() || d.name.to_lowercase().contains(&f) || d.id.to_string() == f)
            .collect();
        let key = self.sort;
        let desc = key.descending() != self.reverse;
        v.sort_by(|a, b| {
            use std::cmp::Ordering;
            let o = match key {
                SortKey::Cpu => a.cpu_pct.total_cmp(&b.cpu_pct),
                SortKey::Mem => a.mem.cmp(&b.mem),
                SortKey::Net => a.net_bps().total_cmp(&b.net_bps()),
                SortKey::Disk => a.disk_bps().total_cmp(&b.disk_bps()),
                SortKey::Lat => a.lat_us().unwrap_or(-1.0).total_cmp(&b.lat_us().unwrap_or(-1.0)),
                SortKey::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
                SortKey::Id => a.id.cmp(&b.id),
            };
            let o = if desc { o.reverse() } else { o };
            // Stable tie-break so equal rows don't jitter between frames.
            if o == Ordering::Equal {
                a.id.cmp(&b.id)
            } else {
                o
            }
        });
        if self.dom0_first {
            if let Some(i) = v.iter().position(|d| d.id == 0) {
                let d0 = v.remove(i);
                v.insert(0, d0);
            }
        }
        v
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

    fn cycle_sort(&mut self, fwd: bool) {
        let i = SortKey::ALL.iter().position(|k| *k == self.sort).unwrap_or(0);
        let n = SortKey::ALL.len();
        self.sort = SortKey::ALL[if fwd { (i + 1) % n } else { (i + n - 1) % n }];
        self.reverse = false;
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
        if self.help {
            self.help = false;
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
            KeyCode::Char('c') => self.sort = SortKey::Cpu,
            KeyCode::Char('m') => self.sort = SortKey::Mem,
            KeyCode::Char('n') => self.sort = SortKey::Net,
            KeyCode::Char('d') => self.sort = SortKey::Disk,
            KeyCode::Char('l') => self.sort = SortKey::Lat,
            KeyCode::Char('0') => self.dom0_first = !self.dom0_first,
            KeyCode::Char('/') | KeyCode::Char('f') => self.filter_edit = true,
            KeyCode::Char('p') => self.paused = !self.paused,
            KeyCode::Char('t') => self.theme = (self.theme + 1) % THEMES.len(),
            KeyCode::Char('T') => self.theme = (self.theme + THEMES.len() - 1) % THEMES.len(),
            KeyCode::Char('+') | KeyCode::Char('=') => self.set_interval(self.interval.as_millis() as u64 + 250),
            KeyCode::Char('-') => self.set_interval((self.interval.as_millis() as u64).saturating_sub(250)),
            KeyCode::Char('?') | KeyCode::Char('h') | KeyCode::F(1) => self.help = true,
            KeyCode::Char(c @ '1'..='4') => {
                let i = c as usize - '1' as usize;
                self.show[i] = !self.show[i];
            }
            _ => {}
        }
    }

    fn set_interval(&mut self, ms: u64) {
        self.interval = Duration::from_millis(ms.clamp(250, 10_000));
        self.next_sample = Instant::now() + self.interval;
    }

    pub fn on_mouse(&mut self, m: MouseEvent) {
        match m.kind {
            MouseEventKind::ScrollUp => self.move_sel(-1),
            MouseEventKind::ScrollDown => self.move_sel(1),
            MouseEventKind::Down(MouseButton::Left) => {
                let r = self.table_rows;
                if !r.contains(Position::new(m.column, m.row)) {
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
}
