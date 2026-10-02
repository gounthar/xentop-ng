//! Domain details (`⏎`): vCPUs, memory, disks and networks of one domain.

use super::columns;
use super::sr::short_sr;
use super::widgets::*;
use super::{bold, boxed, dim, lat_color, steal_color, trunc_left};
use crate::app::App;
use crate::fmt;
use crate::model::{Backing, DomRates, DomState};
use crate::theme::Theme;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Modifier, Style};
use ratatui::text::{Line, Span};
use ratatui::widgets::Widget;

/// Width of one vCPU cell in the detail panel: label, meter, load, and
/// steal when the hypervisor reports it per vCPU.
fn vcpu_cell_w(d: &DomRates) -> usize {
    if d.vcpu_steal_pct.iter().any(Option::is_some) {
        31
    } else {
        23
    }
}

/// Rows the detail panel wants when stacked (borders included).
pub(super) fn detail_height(d: &DomRates, width: u16) -> u16 {
    let per_row = ((width as usize).saturating_sub(2) / vcpu_cell_w(d)).max(1);
    let section = |n: usize| if n > 0 { 2 + n } else { 0 };
    // One more line for the VM UUID, and one under each mapped disk.
    let extra =
        d.vm_uuid.is_some() as usize + d.vbds.iter().filter(|v| backing_line_wanted(&v.backing)).count();
    (2 + 1
        + 1
        + 3
        + extra
        + d.vcpu_pct.len().div_ceil(per_row)
        + section(d.vbds.len())
        + section(d.nets.len())) as u16
}

fn backing_line_wanted(b: &Backing) -> bool {
    b.sr.is_some() || b.vdi.is_some() || b.path.is_some()
}

/// Balloon target, when it is meaningfully away from current memory (a
/// few MiB of difference is just accounting noise).
fn balloon_target(d: &DomRates) -> Option<u64> {
    let t = d.mem_target?;
    let diff = t.abs_diff(d.mem);
    (diff > 32 << 20 && diff * 50 > t.max(d.mem)).then_some(t)
}

/// The line under a disk row saying what backs it.
fn backing_line(th: &Theme, b: &Backing, w: usize) -> Line<'static> {
    let mut sp = vec![dim(th, "      └ ")];
    if b.sr.is_some() || b.vdi.is_some() {
        sp.push(dim(th, "sr "));
        sp.push(Span::styled(
            match (&b.sr_name, &b.sr) {
                (Some(n), _) => fmt::trunc(n, 24),
                (None, Some(s)) => short_sr(s, 8),
                (None, None) => "?".into(),
            },
            Style::new().fg(th.fg),
        ));
        if let Some(k) = &b.sr_kind {
            sp.push(dim(th, format!(" {k}")));
        }
        if let Some(v) = &b.vdi {
            sp.push(dim(th, "  vdi "));
            // The name when xapi has one; the UUID stays, for `xe`.
            if let Some(n) = &b.vdi_name {
                sp.push(Span::styled(fmt::trunc(n, 32), Style::new().fg(th.fg)));
                sp.push(dim(th, format!("  {v}")));
            } else {
                sp.push(Span::styled(v.clone(), Style::new().fg(th.fg)));
            }
        } else if let Some(p) = &b.path {
            sp.push(dim(th, "  "));
            sp.push(Span::styled(
                trunc_left(p, w.saturating_sub(30)),
                Style::new().fg(th.fg),
            ));
        }
    } else if let Some(p) = &b.path {
        sp.push(Span::styled(
            trunc_left(p, w.saturating_sub(8)),
            Style::new().fg(th.fg),
        ));
    }
    Line::from(sp)
}

pub(super) fn detail_box(buf: &mut Buffer, app: &App, d: &DomRates, area: Rect) {
    let th = app.theme();
    let state = match d.state {
        // Same rule as the list: activity over the interval, not the
        // instantaneous running/blocked flag.
        Some(DomState::Running) | Some(DomState::Blocked) if d.cpu_pct >= 5.0 => "running",
        Some(DomState::Running) | Some(DomState::Blocked) => "idle",
        Some(s) => s.label(),
        None => "?",
    };
    let block = boxed(
        th,
        "▸ ",
        &d.name,
        vec![
            dim(th, format!("id {}  ", d.id)),
            Span::styled(state, Style::new().fg(th.fg)),
            dim(th, "  esc close"),
        ],
    )
    .border_style(Style::new().fg(th.key));
    let inner = block.inner(area);
    block.render(area, buf);
    let w = inner.width as usize;
    if w < 20 || inner.height < 3 {
        return;
    }
    let bottom = inner.y + inner.height;
    let mut y = inner.y;
    let hist = app.hist.doms.get(&d.id);
    macro_rules! line {
        ($l:expr) => {
            if y < bottom {
                put(buf, inner.x, y, inner.width, &$l);
                y += 1;
            }
        };
    }

    // Summary.
    line!(Line::from(vec![
        dim(th, "cpu "),
        bold(
            fmt::pct(d.cpu_pct),
            th.cpu.at(d.cpu_pct / (d.vcpus_online.max(1) * 100) as f64)
        ),
        dim(
            th,
            format!(" of {}/{} vCPU online   mem ", d.vcpus_online, d.vcpu_pct.len())
        ),
        bold(fmt::bytes(d.mem as f64), th.mem.at(0.7)),
        dim(th, format!(" / {}", fmt::bytes(d.max_mem as f64))),
        match balloon_target(d) {
            Some(t) => Span::styled(
                format!(" (balloon target {})", fmt::bytes(t as f64)),
                Style::new().fg(th.warn)
            ),
            None => Span::raw(""),
        },
        dim(th, "   steal "),
        Span::styled(
            d.steal_pct.map(fmt::pct).unwrap_or_else(|| "-".into()),
            Style::new().fg(steal_color(th, d.steal_pct)),
        ),
    ]));
    if let Some(u) = &d.vm_uuid {
        line!(Line::from(vec![
            dim(th, "vm uuid "),
            Span::styled(u.clone(), Style::new().fg(th.fg)),
        ]));
    }

    // Memory over time, one compact line: a sparkline against the
    // domain's maximum and the range seen, so ballooning shows. Most
    // domains never change; say so instead of drawing a solid bar.
    if let Some(h) = hist {
        let lw = w.saturating_sub(4 + 26);
        if y < bottom {
            let data = h.mem.tail(lw.max(1));
            let (lo, hi) = data
                .iter()
                .fold((f64::MAX, 0.0f64), |(lo, hi), &v| (lo.min(v), hi.max(v)));
            let secs = (data.len() as f64 * app.interval.as_secs_f64()).round();
            let mut sp = vec![dim(th, "mem ")];
            if hi - lo < 1024.0 * 1024.0 || lw < 8 {
                sp.push(dim(th, format!("steady at {} for {secs:.0}s", fmt::bytes(hi))));
            } else {
                let cap = (d.max_mem as f64).max(hi);
                sp.extend(sparkline(&data, cap, lw, &th.mem, th.meter_empty));
                sp.push(dim(
                    th,
                    format!("  {} – {} in {secs:.0}s", fmt::bytes(lo), fmt::bytes(hi)),
                ));
            }
            line!(Line::from(sp));
        }
    }

    // CPU history graph.
    let gh = if inner.height >= 30 { 6 } else { 3 };
    if y + gh <= bottom {
        let data = hist.map(|h| h.cpu.tail(w * 2)).unwrap_or_default();
        let cap = (d.vcpus_online.max(1) * 100) as f64;
        area_graph(
            buf,
            Rect::new(inner.x, y, inner.width, gh),
            &data,
            cap,
            Paint::Height(&th.cpu),
        );
        y += gh;
    }

    // vCPUs, with their steal time when known.
    let cw = vcpu_cell_w(d);
    let with_steal = cw > 23;
    let per_row = (w / cw).max(1);
    for chunk in d.vcpu_pct.chunks(per_row).enumerate() {
        let (ci, vals) = chunk;
        let mut sp = Vec::new();
        for (k, v) in vals.iter().enumerate() {
            let i = ci * per_row + k;
            sp.push(dim(th, format!("v{i:<2} ")));
            sp.extend(meter(v / 100.0, 12, &th.cpu, th.meter_empty));
            sp.push(Span::styled(
                format!("{:>5.0}%", v),
                Style::new().fg(th.cpu.at(v / 100.0)),
            ));
            if with_steal {
                let st = d.vcpu_steal_pct.get(i).copied().flatten();
                sp.push(dim(th, " st"));
                sp.push(Span::styled(
                    format!("{:>5}", st.map(fmt::pct).unwrap_or_else(|| "-".into())),
                    Style::new().fg(steal_color(th, st)),
                ));
            }
            sp.push(Span::raw("  "));
        }
        line!(Line::from(sp));
    }

    // Disks.
    if !d.vbds.is_empty() && y + 2 <= bottom {
        line!(Line::from(dim(th, format!("{:─<w$}", "── disks "))));
        line!(Line::from(Span::styled(
            format!(
                "{:<6}{:<9}{:>7}{:>7}{:>8}{:>8}{:>8}{:>8}{:>5}",
                "dev", "backend", "r/s", "w/s", "read", "write", "r lat", "w lat", "err"
            ),
            Style::new().fg(th.dim).add_modifier(Modifier::BOLD),
        )));
        for v in &d.vbds {
            line!(Line::from(vec![
                Span::styled(format!("{:<6}", v.name), Style::new().fg(th.fg)),
                dim(th, format!("{:<9}", v.kind.map(|k| k.label()).unwrap_or("?"))),
                Span::styled(
                    format!(
                        "{:>7}",
                        if v.stats_valid {
                            fmt::count(v.rd_iops)
                        } else {
                            "-".into()
                        }
                    ),
                    Style::new().fg(th.rd.at(1.0))
                ),
                Span::styled(
                    format!(
                        "{:>7}",
                        if v.stats_valid {
                            fmt::count(v.wr_iops)
                        } else {
                            "-".into()
                        }
                    ),
                    Style::new().fg(th.wr.at(1.0))
                ),
                Span::styled(
                    format!(
                        "{:>8}",
                        if v.stats_valid {
                            fmt::rate(v.rd_bps)
                        } else {
                            "-".into()
                        }
                    ),
                    Style::new().fg(th.rd.at(1.0))
                ),
                Span::styled(
                    format!(
                        "{:>8}",
                        if v.stats_valid {
                            fmt::rate(v.wr_bps)
                        } else {
                            "-".into()
                        }
                    ),
                    Style::new().fg(th.wr.at(1.0))
                ),
                Span::styled(
                    format!("{:>8}", columns::lat_text(v.rd_lat_us)),
                    Style::new().fg(lat_color(th, v.rd_lat_us))
                ),
                Span::styled(
                    format!("{:>8}", columns::lat_text(v.wr_lat_us)),
                    Style::new().fg(lat_color(th, v.wr_lat_us))
                ),
                Span::styled(
                    format!(
                        "{:>5}",
                        if v.collection_error {
                            "read!".into()
                        } else {
                            v.errors.to_string()
                        }
                    ),
                    Style::new().fg(if v.errors > 0 { th.bad } else { th.dim }),
                ),
            ]));
            if backing_line_wanted(&v.backing) {
                line!(backing_line(th, &v.backing, w));
            }
        }
    }

    // Network.
    if !d.nets.is_empty() && y + 2 <= bottom {
        line!(Line::from(dim(th, format!("{:─<w$}", "── network "))));
        line!(Line::from(Span::styled(
            format!(
                "{:<9}{:>9}{:>9}{:>9}{:>9}{:>8}{}",
                "vif",
                "rx",
                "tx",
                "rx pps",
                "tx pps",
                "err/drp",
                if d.nets.iter().any(|n| n.network.is_some()) {
                    "  network"
                } else {
                    ""
                }
            ),
            Style::new().fg(th.dim).add_modifier(Modifier::BOLD),
        )));
        for n in &d.nets {
            line!(Line::from(vec![
                Span::styled(
                    format!("{:<9}", format!("vif{}.{}", d.id, n.id)),
                    Style::new().fg(th.fg)
                ),
                Span::styled(
                    format!("{:>9}", format!("{}/s", fmt::rate(n.rx_bps))),
                    Style::new().fg(th.rx.at(1.0))
                ),
                Span::styled(
                    format!("{:>9}", format!("{}/s", fmt::rate(n.tx_bps))),
                    Style::new().fg(th.tx.at(1.0))
                ),
                Span::styled(format!("{:>9}", fmt::count(n.rx_pps)), Style::new().fg(th.fg)),
                Span::styled(format!("{:>9}", fmt::count(n.tx_pps)), Style::new().fg(th.fg)),
                Span::styled(
                    format!("{:>8}", format!("{}/{}", n.errs, n.drops)),
                    Style::new().fg(if n.errs + n.drops > 0 { th.warn } else { th.dim }),
                ),
                dim(
                    th,
                    n.network.as_deref().map(|s| format!("  {s}")).unwrap_or_default()
                ),
            ]));
        }
    }
}
