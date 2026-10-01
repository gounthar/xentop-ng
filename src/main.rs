mod app;
mod fmt;
mod history;
mod model;
mod source;
mod term;
mod theme;
mod ui;

use anyhow::{bail, Context, Result};
use app::App;
use ratatui::crossterm::event::{self, Event, KeyEventKind};
use source::{
    demo::{DemoConfig, DemoSource},
    xenstat::XenstatSource,
    Source,
};
use std::io::Write;
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
        --theme NAME      btop, xcp-ng, dracula, gruvbox
        --colors MODE     truecolor or 256 (default: truecolor, 256 on the
                          Linux console)
    -b, --batch           print one JSON object per interval instead of the UI
    -n, --iterations N    stop after N samples (batch mode)
    -h, --help            this help
    -V, --version         print version
";

struct Opts {
    delay: Duration,
    demo: bool,
    demo_cfg: DemoConfig,
    lib: Option<String>,
    theme: usize,
    ansi256: bool,
    batch: bool,
    iterations: Option<u64>,
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

fn parse_args() -> Result<Opts> {
    let mut o = Opts {
        delay: Duration::from_secs(1),
        demo: false,
        demo_cfg: DemoConfig::default(),
        lib: None,
        theme: 0,
        // The Linux VT has no 24-bit colour; most other terminals do.
        ansi256: std::env::var("TERM").is_ok_and(|t| t == "linux"),
        batch: false,
        iterations: None,
    };
    let mut args = std::env::args().skip(1);
    while let Some(a) = args.next() {
        let mut val = |name: &str| args.next().with_context(|| format!("{name} needs a value"));
        match a.as_str() {
            "-d" | "--delay" => {
                let s: f64 = val(&a)?.parse().context("bad --delay")?;
                o.delay = Duration::from_secs_f64(s.clamp(0.1, 60.0));
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
            "--colors" => match val(&a)?.as_str() {
                "256" => o.ansi256 = true,
                "truecolor" | "24bit" => o.ansi256 = false,
                m => bail!("--colors: expected truecolor or 256, got {m}"),
            },
            "--theme" => {
                let t = val(&a)?;
                o.theme = theme::by_name(&t).with_context(|| format!("unknown theme {t}"))?;
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
        std::thread::sleep(o.delay);
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

fn run_ui(src: Box<dyn Source>, o: &Opts) -> Result<()> {
    let mut app = App::new(src, o.delay, o.theme);
    app.ansi256 = o.ansi256;
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
        app.next_sample = Instant::now() + o.delay;
    }

    let (mut term, _guard) = term::Guard::enter()?;
    (|| -> Result<()> {
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
    })()
}

fn main() -> Result<()> {
    let o = parse_args()?;
    let src: Box<dyn Source> = if o.demo {
        Box::new(DemoSource::new(&o.demo_cfg))
    } else {
        Box::new(XenstatSource::open(o.lib.as_deref())?)
    };
    if o.batch {
        batch(src, &o)
    } else {
        run_ui(src, &o)
    }
}
