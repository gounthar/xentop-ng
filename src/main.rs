mod app;
mod config;
mod fmt;
mod history;
mod model;
mod source;
mod term;
mod theme;
mod ui;
mod xentop_compat;

use anyhow::{bail, Context, Result};
use app::App;
use config::{ColorMode, Prefs};
use ratatui::crossterm::event::{self, Event, KeyEventKind};
use source::{
    demo::{DemoConfig, DemoSource},
    xenstat::XenstatSource,
    Source,
};
use std::io::Write;
use std::path::PathBuf;
use std::time::{Duration, Instant};

const USAGE: &str = "\
xentop-ng — a modern resource monitor for Xen

USAGE:
    xentop-ng [OPTIONS]

OPTIONS:
    -d, --delay SECS      refresh interval (default 1.0)
        --demo            simulated host, no Xen needed
        --demo-cpus N     demo: physical CPUs (default 16)
        --demo-mem SIZE   demo: RAM, e.g. 512G, 1T (default 128G)
        --demo-load PCT   demo: average host CPU load to aim for (default 55)
        --demo-mem-use PCT
                          demo: share of RAM given to VMs (default 80)
        --demo-stock      demo: behave like a stock libxenstat with no
                          fallbacks (no pCPU detail, no disk latency)
                          Any --demo-* option implies --demo.
        --lib PATH        libxenstat to load (default: search, LD_LIBRARY_PATH first).
                          As root, must be root-owned and not group/world-writable.
        --theme NAME      btop, xcp-ng, dracula, gruvbox, colorblind
        --domains-only    start with only the domain list (key 5 toggles)
        --colors MODE     truecolor, 256 or mono (default: truecolor; 256 on
                          the Linux console; mono if NO_COLOR is set)
        --config PATH     preferences file (default:
                          $XDG_CONFIG_HOME/xentop-ng/config.toml or
                          ~/.config/xentop-ng/config.toml)
        --no-config       don't read or write the preferences file
    -b, --batch           print one JSON object per interval instead of the UI
    -n, --iterations N    stop after N samples (batch mode)
    -h, --help            this help
    -V, --version         print version
        --xentop ARGS...  behave like xentop with xentop's options (also
                          when invoked as `xentop`); `--xentop -b` prints
                          xentop's batch format
";

struct Opts {
    /// Command-line settings are `None` when not given, so the config file
    /// (UI only) or the defaults apply.
    delay: Option<Duration>,
    demo: bool,
    demo_cfg: DemoConfig,
    lib: Option<String>,
    theme: Option<usize>,
    colors: Option<ColorMode>,
    domains_only: bool,
    config: Option<PathBuf>,
    no_config: bool,
    batch: bool,
    iterations: Option<u64>,
    dom0_first: bool,
}

/// "512G", "1T", "1TiB", "64g" -> bytes (binary units; bare number = GiB).
fn parse_size(s: &str) -> Option<u64> {
    let s = s.trim().trim_end_matches(['B', 'b']).trim_end_matches(['i', 'I']);
    let (num, unit) = s.split_at(s.find(|c: char| c.is_ascii_alphabetic()).unwrap_or(s.len()));
    let n: f64 = num.parse().ok()?;
    let shift = match unit.to_ascii_uppercase().as_str() {
        "" | "G" => 30,
        "M" => 20,
        "T" => 40,
        "P" => 50,
        _ => return None,
    };
    let v = n * (1u64 << shift) as f64;
    (v >= (1u64 << 30) as f64).then_some(v as u64)
}

fn parse_args(args: Vec<String>) -> Result<Opts> {
    let mut o = Opts {
        delay: None,
        demo: false,
        demo_cfg: DemoConfig::default(),
        lib: None,
        theme: None,
        colors: None,
        domains_only: false,
        config: None,
        no_config: false,
        batch: false,
        iterations: None,
        dom0_first: false,
    };
    let mut args = args.into_iter();
    while let Some(a) = args.next() {
        let mut val = |name: &str| args.next().with_context(|| format!("{name} needs a value"));
        match a.as_str() {
            "-d" | "--delay" => {
                let s: f64 = val(&a)?.parse().context("bad --delay")?;
                if !s.is_finite() {
                    bail!("bad --delay");
                }
                o.delay = Some(Duration::from_secs_f64(
                    s.clamp(config::MIN_INTERVAL, config::MAX_INTERVAL),
                ));
            }
            "--demo" => o.demo = true,
            "--demo-mem" => {
                o.demo = true;
                let v = val(&a)?;
                o.demo_cfg.mem = parse_size(&v).with_context(|| format!("bad --demo-mem {v}"))?;
            }
            "--demo-cpus" => {
                o.demo = true;
                o.demo_cfg.pcpus = val(&a)?.parse::<u32>().context("bad --demo-cpus")?.clamp(1, 4096);
            }
            "--demo-stock" => {
                o.demo = true;
                o.demo_cfg.stock = true;
            }
            "--demo-load" | "--demo-mem-use" => {
                o.demo = true;
                let pct: f64 = val(&a)?
                    .trim_end_matches('%')
                    .parse()
                    .with_context(|| format!("bad {a}"))?;
                let f = (pct / 100.0).clamp(0.01, 1.0);
                if a == "--demo-load" {
                    o.demo_cfg.cpu_load = f;
                } else {
                    o.demo_cfg.mem_use = f;
                }
            }
            "--lib" => o.lib = Some(val(&a)?),
            "--colors" => {
                let m = val(&a)?;
                o.colors = Some(
                    ColorMode::parse(&m)
                        .with_context(|| format!("--colors: expected truecolor, 256 or mono, got {m}"))?,
                );
            }
            "--config" => o.config = Some(PathBuf::from(val(&a)?)),
            "--no-config" => o.no_config = true,
            "--domains-only" => o.domains_only = true,
            "--theme" => {
                let t = val(&a)?;
                o.theme = Some(theme::by_name(&t).with_context(|| format!("unknown theme {t}"))?);
            }
            "-b" | "--batch" => o.batch = true,
            "-n" | "--iterations" => o.iterations = Some(val(&a)?.parse().context("bad -n")?),
            "-h" | "--help" => {
                print!("{USAGE}");
                std::process::exit(0);
            }
            "-V" | "--version" => {
                println!("xentop-ng {}", env!("CARGO_PKG_VERSION"));
                std::process::exit(0);
            }
            _ => bail!("unknown argument {a}\n\n{USAGE}"),
        }
    }
    Ok(o)
}

fn batch(mut src: Box<dyn Source>, o: &Opts) -> Result<()> {
    let mut prev = src.sample()?;
    let mut out = std::io::stdout().lock();
    let mut n = 0;
    loop {
        std::thread::sleep(o.delay.unwrap_or(Duration::from_secs(1)));
        let cur = src.sample()?;
        let r = model::compute(&prev, &cur);
        let mut v = serde_json::to_value(&r)?;
        v["sources"] = serde_json::to_value(src.status())?;
        serde_json::to_writer(&mut out, &v)?;
        writeln!(out)?;
        out.flush()?;
        prev = cur;
        n += 1;
        if o.iterations.is_some_and(|max| n >= max) {
            return Ok(());
        }
    }
}

/// Preferences for this run: the config file (unless `--no-config`), then
/// command-line flags on top. Returns them with the config state and any
/// warnings about the file.
fn startup_prefs(o: &Opts) -> (Prefs, Option<app::ConfigState>, Vec<String>) {
    let path = if o.no_config {
        None
    } else {
        o.config.clone().or_else(config::default_path)
    };
    let (file, warnings) = match &path {
        Some(p) => config::load(p),
        None => (Prefs::default(), Vec::new()),
    };
    let mut start = file.clone();
    if let Some(t) = o.theme {
        start.theme = t;
    }
    if let Some(c) = o.colors {
        start.colors = c;
    }
    if let Some(d) = o.delay {
        start.interval = d;
    }
    if o.domains_only {
        start.boxes = [false; 4];
    }
    // xentop's -z (through the drop-in mode).
    if o.dom0_first {
        start.dom0_first = true;
    }
    let state = path.map(|path| app::ConfigState {
        path,
        file,
        start: start.clone(),
    });
    (start, state, warnings)
}

fn run_ui(src: Box<dyn Source>, o: &Opts) -> Result<()> {
    let (prefs, cfg, warnings) = startup_prefs(o);
    let mut app = App::new(src, prefs.interval, prefs.theme);
    let term = std::env::var("TERM").ok();
    // no-color.org: set and not empty.
    let no_color = std::env::var_os("NO_COLOR").is_some_and(|v| !v.is_empty());
    app.apply_prefs(&prefs, term.as_deref(), no_color);
    if let Some(cfg) = &cfg {
        if !warnings.is_empty() {
            let more = if warnings.len() > 1 {
                format!(" (+{} more)", warnings.len() - 1)
            } else {
                String::new()
            };
            let name = cfg.path.file_name().unwrap_or_default().to_string_lossy();
            app.notify(format!("{name}: {}{more}", warnings[0]), true);
        }
    }
    app.config = cfg;
    let history = app.source.warmup(600);
    if history.is_empty() {
        // Two quick samples so the first frame already has rates.
        app.tick();
        std::thread::sleep(Duration::from_millis(250));
        app.next_sample = Instant::now();
        app.tick();
    } else {
        for snap in history {
            app.ingest(snap);
        }
        app.status = app.source.status();
        app.next_sample = Instant::now() + app.interval;
    }

    let (mut term, guard) = term::Guard::enter()?;
    let res = (|| -> Result<()> {
        while !app.quit {
            term.draw(|f| ui::draw(f, &mut app))?;
            // Wake at the next sample, or once a second for the clock.
            let wait = app
                .next_sample
                .saturating_duration_since(Instant::now())
                .min(Duration::from_millis(1000));
            if event::poll(wait)? {
                match event::read()? {
                    Event::Key(k) if k.kind != KeyEventKind::Release => app.on_key(k),
                    Event::Mouse(m) => app.on_mouse(m),
                    _ => {}
                }
            } else {
                app.tick();
            }
        }
        Ok(())
    })();
    // The terminal is back to normal before anything is printed.
    drop(guard);
    if let Some(Err(e)) = app.save_on_quit() {
        if let Some(cfg) = &app.config {
            eprintln!("xentop-ng: couldn't save {}: {e}", cfg.path.display());
        }
    }
    res
}

fn main() -> Result<()> {
    let (args, xentop_args) = xentop_compat::split_args(std::env::args());
    let mut o = parse_args(args)?;
    let xentop = xentop_args.map(|(prog, a)| xentop_compat::parse_or_exit(&prog, &a));
    if let Some(x) = &xentop {
        o.delay = x.ui_delay().or(o.delay);
        o.dom0_first = x.dom0_first;
    }
    let src: Box<dyn Source> = if o.demo {
        Box::new(DemoSource::new(&o.demo_cfg))
    } else {
        Box::new(XenstatSource::open(o.lib.as_deref())?)
    };
    if let Some(x) = xentop.filter(|x| x.batch) {
        xentop_compat::batch(src, &x)
    } else if o.batch {
        batch(src, &o)
    } else {
        run_ui(src, &o)
    }
}
