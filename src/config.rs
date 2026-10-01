//! Preferences file: `$XDG_CONFIG_HOME/xentop-ng/config.toml` (by default
//! `~/.config/xentop-ng/config.toml`).
//!
//! Loading never fails: a missing file means defaults, a malformed file or
//! a bad value means defaults for what couldn't be read, plus a warning the
//! UI shows in its header. Unknown keys are ignored, so files written by a
//! newer version still load. Saving writes a temporary file (mode 0600) and
//! renames it over the old one.

use crate::theme::THEMES;
use crate::ui::columns::{self, COLUMNS};
use serde::Serialize;
use std::io::Write;
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};
use std::time::Duration;

pub const BOXES: [&str; 4] = ["cpu", "mem", "net", "disk"];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum ColorMode {
    /// 256 colours on the Linux console, truecolor elsewhere; monochrome if
    /// `NO_COLOR` is set.
    Auto,
    Truecolor,
    Ansi256,
    Mono,
}

impl ColorMode {
    pub fn parse(s: &str) -> Option<Self> {
        Some(match s {
            "auto" => ColorMode::Auto,
            "truecolor" | "24bit" => ColorMode::Truecolor,
            "256" => ColorMode::Ansi256,
            "mono" | "none" | "no" => ColorMode::Mono,
            _ => return None,
        })
    }
    pub fn name(self) -> &'static str {
        match self {
            ColorMode::Auto => "auto",
            ColorMode::Truecolor => "truecolor",
            ColorMode::Ansi256 => "256",
            ColorMode::Mono => "mono",
        }
    }
    /// (256 colours, monochrome) for this mode, given `$TERM` and whether
    /// `$NO_COLOR` is set. An explicit mode wins over `NO_COLOR`, as
    /// no-color.org asks.
    pub fn resolve(self, term: Option<&str>, no_color: bool) -> (bool, bool) {
        match self {
            ColorMode::Auto => (term == Some("linux"), no_color),
            ColorMode::Truecolor => (false, false),
            ColorMode::Ansi256 => (true, false),
            ColorMode::Mono => (false, true),
        }
    }
}

/// Everything the config file remembers.
#[derive(Clone, PartialEq, Debug)]
pub struct Prefs {
    /// Index into `THEMES`.
    pub theme: usize,
    pub colors: ColorMode,
    pub interval: Duration,
    /// cpu, mem, net, disk boxes; all off = domains only.
    pub boxes: [bool; 4],
    pub sort: &'static str,
    pub reverse: bool,
    pub dom0_first: bool,
    /// Every registry column, in display order, with whether it's shown.
    pub columns: Vec<(&'static str, bool)>,
}

impl Default for Prefs {
    fn default() -> Self {
        Prefs {
            theme: 0,
            colors: ColorMode::Auto,
            interval: Duration::from_secs(1),
            boxes: [true; 4],
            sort: "cpu",
            reverse: false,
            dom0_first: false,
            columns: columns::defaults(),
        }
    }
}

pub const MIN_INTERVAL: f64 = 0.1;
pub const MAX_INTERVAL: f64 = 60.0;

/// The file as written, field by field.
#[derive(Serialize)]
struct File<'a> {
    theme: &'a str,
    colors: &'a str,
    interval: f64,
    boxes: Vec<&'a str>,
    sort: &'a str,
    reverse: bool,
    dom0_first: bool,
    columns: Vec<&'a str>,
    hidden_columns: Vec<&'a str>,
}

const HEADER: &str = "\
# xentop-ng preferences. Saved on quit when something was changed in the
# UI, or right away with W. Delete this file to get the defaults back.
#
# theme:    btop, xcp-ng, dracula, gruvbox, colorblind
# colors:   auto, truecolor, 256, mono (auto honours NO_COLOR)
# interval: seconds between samples
# boxes:    cpu, mem, net, disk; [] = domains only
# sort:     a column id below, or net / disk (totals)
# columns:  display order (o in the UI); hidden_columns: not shown
";

impl Prefs {
    pub fn to_toml(&self) -> String {
        let f = File {
            theme: THEMES.get(self.theme).map(|t| t.name).unwrap_or(THEMES[0].name),
            colors: self.colors.name(),
            interval: (self.interval.as_secs_f64() * 1000.0).round() / 1000.0,
            boxes: BOXES
                .iter()
                .zip(self.boxes)
                .filter(|(_, on)| *on)
                .map(|(b, _)| *b)
                .collect(),
            sort: self.sort,
            reverse: self.reverse,
            dom0_first: self.dom0_first,
            columns: self.columns.iter().map(|c| c.0).collect(),
            hidden_columns: self.columns.iter().filter(|c| !c.1).map(|c| c.0).collect(),
        };
        // Can't fail: plain strings, numbers, booleans and arrays.
        let body = toml::to_string(&f).unwrap_or_default();
        format!("{HEADER}\n{body}")
    }

    /// Parse a config file. Never fails: whatever can't be used falls back
    /// to its default and adds a warning.
    pub fn from_toml(text: &str) -> (Prefs, Vec<String>) {
        let mut p = Prefs::default();
        let mut warn = Vec::new();
        let t: toml::Table = match text.parse() {
            Ok(t) => t,
            Err(e) => {
                let first = e.to_string();
                let first = first.lines().next().unwrap_or("syntax error").trim().to_string();
                warn.push(format!("unreadable, using defaults ({first})"));
                return (p, warn);
            }
        };
        let s = |k: &str, warn: &mut Vec<String>| -> Option<String> {
            match t.get(k)? {
                toml::Value::String(s) => Some(s.clone()),
                _ => {
                    warn.push(format!("{k}: expected a string"));
                    None
                }
            }
        };
        let b = |k: &str, warn: &mut Vec<String>| -> Option<bool> {
            match t.get(k)? {
                toml::Value::Boolean(b) => Some(*b),
                _ => {
                    warn.push(format!("{k}: expected true or false"));
                    None
                }
            }
        };
        let list = |k: &str, warn: &mut Vec<String>| -> Option<Vec<String>> {
            match t.get(k)? {
                toml::Value::Array(a) => {
                    Some(a.iter().filter_map(|v| v.as_str().map(String::from)).collect())
                }
                _ => {
                    warn.push(format!("{k}: expected a list of names"));
                    None
                }
            }
        };

        if let Some(v) = s("theme", &mut warn) {
            match crate::theme::by_name(&v) {
                Some(i) => p.theme = i,
                None => warn.push(format!("unknown theme \"{v}\"")),
            }
        }
        if let Some(v) = s("colors", &mut warn) {
            match ColorMode::parse(&v) {
                Some(c) => p.colors = c,
                None => warn.push(format!("unknown colors \"{v}\"")),
            }
        }
        match t.get("interval") {
            None => {}
            Some(v) => match v.as_float().or(v.as_integer().map(|i| i as f64)) {
                Some(x) if x.is_finite() => {
                    if !(MIN_INTERVAL..=MAX_INTERVAL).contains(&x) {
                        warn.push(format!("interval {x} out of {MIN_INTERVAL}..{MAX_INTERVAL}"));
                    }
                    p.interval = Duration::from_secs_f64(x.clamp(MIN_INTERVAL, MAX_INTERVAL));
                }
                _ => warn.push("interval: expected seconds".into()),
            },
        }
        if let Some(v) = list("boxes", &mut warn) {
            p.boxes = [false; 4];
            for name in v {
                match BOXES.iter().position(|b| *b == name) {
                    Some(i) => p.boxes[i] = true,
                    None => warn.push(format!("unknown box \"{name}\"")),
                }
            }
        }
        if let Some(v) = s("sort", &mut warn) {
            match columns::sort_key(&v) {
                Some(k) => p.sort = k,
                None => warn.push(format!("can't sort by \"{v}\"")),
            }
        }
        if let Some(v) = b("reverse", &mut warn) {
            p.reverse = v;
        }
        if let Some(v) = b("dom0_first", &mut warn) {
            p.dom0_first = v;
        }
        let order = list("columns", &mut warn);
        let hidden = list("hidden_columns", &mut warn);
        if order.is_some() || hidden.is_some() {
            p.columns = merge_columns(&order.unwrap_or_default(), &hidden.unwrap_or_default());
        }
        (p, warn)
    }
}

/// Rebuild the full column list from a file's `columns` (display order)
/// and `hidden_columns` (which of them aren't shown). Ids this version
/// doesn't know are skipped (the file may come from a newer one). Columns
/// the file doesn't order are newer than the file, or left out by hand:
/// they go after the column that precedes them in the registry. Columns
/// not in `hidden_columns` are shown, except new ones that are off by
/// default and that the file doesn't mention at all.
pub fn merge_columns(order: &[String], hidden: &[String]) -> Vec<(&'static str, bool)> {
    let known = |id: &str| COLUMNS.iter().find(|c| c.id == id);
    let is_hidden = |c: &columns::Column| !c.locked && hidden.iter().any(|h| h == c.id);
    let mut out: Vec<(&'static str, bool)> = Vec::new();
    for id in order {
        if let Some(c) = known(id) {
            if !out.iter().any(|o| o.0 == c.id) {
                out.push((c.id, !is_hidden(c)));
            }
        }
    }
    for (i, c) in COLUMNS.iter().enumerate() {
        if out.iter().any(|o| o.0 == c.id) {
            continue;
        }
        let at = COLUMNS[..i]
            .iter()
            .rev()
            .find_map(|p| out.iter().position(|o| o.0 == p.id))
            .map(|p| p + 1)
            .unwrap_or(0);
        let on = if hidden.iter().any(|h| h == c.id) {
            c.locked
        } else {
            c.default_on
        };
        out.insert(at, (c.id, on));
    }
    out
}

/// Where the config lives: `--config PATH`, else `$XDG_CONFIG_HOME`, else
/// `$HOME/.config`. None if neither variable is usable.
pub fn default_path() -> Option<PathBuf> {
    let base = std::env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| {
            std::env::var_os("HOME")
                .map(PathBuf::from)
                .filter(|p| p.is_absolute())
                .map(|h| h.join(".config"))
        })?;
    Some(base.join("xentop-ng").join("config.toml"))
}

fn is_root() -> bool {
    // SAFETY: geteuid has no preconditions.
    unsafe { libc::geteuid() == 0 }
}

/// As root, only use files and directories that root owns and nobody else
/// can write. Under `sudo` with a preserved `$HOME`, the path is in another
/// user's hands, and a symlink could point it at some other program's
/// config.toml.
fn untrusted(st: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    is_root() && (st.uid() != 0 || st.mode() & 0o022 != 0)
}

/// Read the config at `path`. A missing file is not an error.
pub fn load(path: &Path) -> (Prefs, Vec<String>) {
    use std::io::Read;
    let open = std::fs::OpenOptions::new()
        .read(true)
        .custom_flags(if is_root() { libc::O_NOFOLLOW } else { 0 })
        .open(path);
    let f = match open {
        Ok(f) => f,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return (Prefs::default(), Vec::new()),
        Err(e) => return (Prefs::default(), vec![format!("can't read it: {e}")]),
    };
    match f.metadata() {
        Ok(st) if untrusted(&st) => {
            return (
                Prefs::default(),
                vec!["ignored: running as root, and root doesn't own it".into()],
            )
        }
        Err(e) => return (Prefs::default(), vec![format!("can't read it: {e}")]),
        Ok(_) => {}
    }
    let mut bytes = Vec::new();
    if let Err(e) = (&f).take(1 << 20).read_to_end(&mut bytes) {
        return (Prefs::default(), vec![format!("can't read it: {e}")]);
    }
    if bytes.len() >= 1 << 20 {
        return (Prefs::default(), vec!["file too large, ignored".into()]);
    }
    match String::from_utf8(bytes) {
        Ok(text) => Prefs::from_toml(&text),
        Err(_) => (Prefs::default(), vec!["not UTF-8, using defaults".into()]),
    }
}

/// Write `prefs` to `path` atomically: a temporary file in the same
/// directory, fsync, rename. Everything happens relative to one open
/// directory handle, so the path can't be switched half-way through.
pub fn save(path: &Path, prefs: &Prefs) -> std::io::Result<()> {
    use std::ffi::CString;
    use std::io::{Error, ErrorKind};
    use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};
    use std::os::unix::ffi::OsStrExt;

    let dir = path
        .parent()
        .filter(|d| !d.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    let name = path
        .file_name()
        .ok_or_else(|| Error::new(ErrorKind::InvalidInput, "no file name"))?;
    std::fs::DirBuilder::new()
        .recursive(true)
        .mode(0o700)
        .create(dir)?;
    let dir_c = CString::new(dir.as_os_str().as_bytes())?;
    // SAFETY: plain syscalls on a NUL-terminated path; the fd is owned below.
    let dfd = unsafe {
        libc::open(
            dir_c.as_ptr(),
            libc::O_RDONLY | libc::O_DIRECTORY | libc::O_NOFOLLOW | libc::O_CLOEXEC,
        )
    };
    if dfd < 0 {
        return Err(Error::last_os_error());
    }
    // SAFETY: dfd is a fresh, valid descriptor we own.
    let dfd = unsafe { OwnedFd::from_raw_fd(dfd) };
    let dirfile = std::fs::File::from(dfd);
    if untrusted(&dirfile.metadata()?) {
        return Err(Error::new(
            ErrorKind::PermissionDenied,
            "running as root, and root doesn't own the directory",
        ));
    }
    let d = dirfile.as_raw_fd();
    let name_c = CString::new(name.as_bytes())?;
    let tmp_c = CString::new(format!(".{}.{}.tmp", name.to_string_lossy(), std::process::id()))?;
    // SAFETY: d is open; the names are NUL-terminated.
    let fd = unsafe {
        libc::openat(
            d,
            tmp_c.as_ptr(),
            libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL | libc::O_NOFOLLOW | libc::O_CLOEXEC,
            0o600 as libc::c_uint,
        )
    };
    if fd < 0 {
        return Err(Error::last_os_error());
    }
    // SAFETY: fd is a fresh, valid descriptor we own.
    let mut f = std::fs::File::from(unsafe { OwnedFd::from_raw_fd(fd) });
    let res = (|| {
        f.write_all(prefs.to_toml().as_bytes())?;
        f.sync_all()?;
        // SAFETY: as above.
        if unsafe { libc::renameat(d, tmp_c.as_ptr(), d, name_c.as_ptr()) } != 0 {
            return Err(Error::last_os_error());
        }
        let _ = dirfile.sync_all();
        Ok(())
    })();
    if res.is_err() {
        // SAFETY: as above.
        unsafe { libc::unlinkat(d, tmp_c.as_ptr(), 0) };
    }
    res
}

/// Of three versions of the preferences (in the file, at startup after
/// command-line overrides, now), what to write back on quit: only what was
/// changed in the UI replaces the file's values. Flags like `--theme` are
/// for this run and don't end up in the file by themselves.
pub fn merge(file: &Prefs, start: &Prefs, now: &Prefs) -> Prefs {
    fn pick<T: PartialEq + Clone>(f: &T, s: &T, n: &T) -> T {
        if n != s {
            n.clone()
        } else {
            f.clone()
        }
    }
    Prefs {
        theme: pick(&file.theme, &start.theme, &now.theme),
        colors: pick(&file.colors, &start.colors, &now.colors),
        interval: pick(&file.interval, &start.interval, &now.interval),
        boxes: pick(&file.boxes, &start.boxes, &now.boxes),
        sort: pick(&file.sort, &start.sort, &now.sort),
        reverse: pick(&file.reverse, &start.reverse, &now.reverse),
        dom0_first: pick(&file.dom0_first, &start.dom0_first, &now.dom0_first),
        columns: pick(&file.columns, &start.columns, &now.columns),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("xentop-ng-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn custom() -> Prefs {
        let mut cols = columns::defaults();
        cols.swap(2, 6);
        cols[3].1 = false;
        Prefs {
            theme: crate::theme::by_name("colorblind").unwrap(),
            colors: ColorMode::Ansi256,
            interval: Duration::from_millis(2500),
            boxes: [true, false, true, false],
            sort: "iops",
            reverse: true,
            dom0_first: true,
            columns: cols,
        }
    }

    #[test]
    fn round_trip() {
        for p in [Prefs::default(), custom()] {
            let text = p.to_toml();
            let (q, warn) = Prefs::from_toml(&text);
            assert!(warn.is_empty(), "{warn:?}\n{text}");
            assert_eq!(p, q, "{text}");
        }
        // Domains only survives too.
        let p = Prefs {
            boxes: [false; 4],
            ..Default::default()
        };
        assert_eq!(Prefs::from_toml(&p.to_toml()).0, p);
    }

    #[test]
    fn save_and_load_atomically_with_private_mode() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tmpdir("save");
        let path = dir.join("sub").join("config.toml");
        assert_eq!(
            load(&path),
            (Prefs::default(), vec![]),
            "missing file = defaults, quietly"
        );
        save(&path, &custom()).unwrap();
        save(&path, &custom()).unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode, 0o600);
        assert_eq!(load(&path), (custom(), vec![]));
        let left: Vec<_> = std::fs::read_dir(path.parent().unwrap()).unwrap().collect();
        assert_eq!(left.len(), 1, "no temporary files left behind");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn malformed_files_never_fail() {
        for text in [
            "this is not toml",
            "theme = ",
            "[[[",
            "\u{0}\u{1}",
            "theme = 42\ncolors = []\ninterval = \"fast\"\nboxes = 1\nsort = true\ncolumns = 3",
        ] {
            let (p, warn) = Prefs::from_toml(text);
            assert_eq!(p, Prefs::default(), "{text:?}");
            assert!(!warn.is_empty(), "{text:?} should warn");
        }
        let dir = tmpdir("bad");
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        std::fs::write(&path, [0xff, 0xfe, 0x00]).unwrap();
        let (p, warn) = load(&path);
        assert_eq!(p, Prefs::default());
        assert_eq!(warn.len(), 1);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn bad_values_fall_back_one_by_one() {
        let (p, warn) = Prefs::from_toml(
            "theme = \"neon\"\nsort = \"mem\"\ninterval = 600\nboxes = [\"cpu\", \"gpu\"]\n\
             future_option = 1\n[future_table]\nx = 1\n",
        );
        assert_eq!(p.theme, 0);
        assert_eq!(p.sort, "mem");
        assert_eq!(p.interval, Duration::from_secs(60), "clamped");
        assert_eq!(p.boxes, [true, false, false, false]);
        assert_eq!(
            warn.len(),
            3,
            "theme, interval, gpu; unknown keys are fine: {warn:?}"
        );
    }

    #[test]
    fn columns_from_older_or_newer_versions() {
        // A file that orders fewer columns (older version) and names one we
        // don't know (newer version).
        let order: Vec<String> = ["name", "id", "steal_from_the_future", "mem", "lat", "cpu"]
            .iter()
            .map(|s| s.to_string())
            .collect();
        let hidden = vec!["lat".to_string(), "name".to_string(), "unknown".to_string()];
        let cols = merge_columns(&order, &hidden);
        assert_eq!(cols.len(), COLUMNS.len(), "every known column exactly once");
        let listed: Vec<_> = cols
            .iter()
            .filter(|c| ["name", "id", "mem", "lat", "cpu"].contains(&c.0))
            .collect();
        assert_eq!(
            listed,
            [
                &("name", true),
                &("id", true),
                &("mem", true),
                &("lat", false),
                &("cpu", true)
            ]
        );
        // Unlisted "state" comes right after "name", as in the registry.
        assert_eq!(cols[1], ("state", true));
        // Not mentioned: default visibility, next to its registry neighbour.
        let iops = cols.iter().position(|c| c.0 == "iops").unwrap();
        assert!(cols[iops].1);
        assert_eq!(cols[iops - 1].0, "disk_wr");
        // Locked columns can't be hidden through the file.
        let (p, w) = Prefs::from_toml("hidden_columns = [\"id\", \"name\", \"state\"]");
        assert!(w.is_empty());
        assert!(p.columns.contains(&("id", true)) && p.columns.contains(&("name", true)));
        assert!(p.columns.contains(&("state", false)));
    }

    #[test]
    fn only_ui_changes_are_written_back() {
        let file = Prefs::default();
        // Started with --theme gruvbox and -d 3.
        let start = Prefs {
            theme: 3,
            interval: Duration::from_secs(3),
            ..file.clone()
        };
        // Then sorted by IOPS in the UI.
        let now = Prefs {
            sort: "iops",
            ..start.clone()
        };
        let out = merge(&file, &start, &now);
        assert_eq!(out.theme, 0, "flag not persisted");
        assert_eq!(out.interval, Duration::from_secs(1));
        assert_eq!(out.sort, "iops");
    }

    /// The example in README.md loads without a single warning.
    #[test]
    fn readme_example_is_valid() {
        let readme = include_str!("../README.md");
        let start = readme.find("```toml\n").expect("toml example in README") + 8;
        let len = readme[start..].find("```").unwrap();
        let (p, warn) = Prefs::from_toml(&readme[start..start + len]);
        assert!(warn.is_empty(), "{warn:?}");
        assert_eq!(p.sort, "iops");
        assert!(p.columns.contains(&("vcpu", false)));
    }

    #[test]
    fn color_modes() {
        assert_eq!(ColorMode::Auto.resolve(Some("linux"), false), (true, false));
        assert_eq!(ColorMode::Auto.resolve(Some("xterm"), true), (false, true));
        assert_eq!(
            ColorMode::Truecolor.resolve(None, true),
            (false, false),
            "explicit beats NO_COLOR"
        );
        for m in [
            ColorMode::Auto,
            ColorMode::Truecolor,
            ColorMode::Ansi256,
            ColorMode::Mono,
        ] {
            assert_eq!(ColorMode::parse(m.name()), Some(m));
        }
    }
}
