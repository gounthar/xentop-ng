//! Drawing primitives: braille area graphs, block meters, sparklines.

use crate::theme::Gradient;
use ratatui::buffer::Buffer;
use ratatui::layout::Rect;
use ratatui::style::{Color, Style};
use ratatui::text::{Line, Span};

// Braille dot bits, top to bottom, for the left and right dot columns.
const LEFT: [u8; 4] = [0x01, 0x02, 0x04, 0x40];
const RIGHT: [u8; 4] = [0x08, 0x10, 0x20, 0x80];

fn braille(bits: u8) -> char {
    char::from_u32(0x2800 + bits as u32).unwrap_or(' ')
}

/// How an area graph is coloured.
pub enum Paint<'a> {
    /// By height within the graph (btop style).
    Height(&'a Gradient),
    /// By the sample's own value, for metrics with absolute thresholds.
    Value(&'a dyn Fn(f64) -> Color),
}

/// Dots for a sample: rounded, but anything above ~3% of the scale still
/// gets one dot so small spikes stay visible, while near-zero noise doesn't
/// paint a dotted "baseline".
fn dots_for(v: f64, max: f64, dots: usize) -> usize {
    if v <= 0.0 || max <= 0.0 {
        return 0;
    }
    let f = (v / max).min(1.0);
    let l = (f * dots as f64).round() as usize;
    if l == 0 && f >= 0.03 {
        1
    } else {
        l.min(dots)
    }
}

/// A filled area graph drawn with braille dots: two samples per cell
/// horizontally, four levels per cell vertically, growing up from the bottom
/// edge. The newest sample sits at the right edge.
pub fn area_graph(buf: &mut Buffer, area: Rect, data: &[f64], max: f64, paint: Paint) {
    let (w, h) = (area.width as usize, area.height as usize);
    if w == 0 || h == 0 {
        return;
    }
    let dots = h * 4;
    let need = w * 2;
    let max = if max > 0.0 { max } else { 1.0 };
    let sample = |sc: usize| -> f64 {
        let idx = data.len() as isize - need as isize + sc as isize;
        if idx < 0 {
            0.0
        } else {
            data[idx as usize]
        }
    };
    for cx in 0..w {
        let (vl, vr) = (sample(cx * 2), sample(cx * 2 + 1));
        let (hl, hr) = (dots_for(vl, max, dots), dots_for(vr, max, dots));
        if hl == 0 && hr == 0 {
            continue;
        }
        for cy in 0..h {
            let mut bits = 0u8;
            for k in 0..4 {
                let lv = (h - 1 - cy) * 4 + (3 - k);
                if lv < hl {
                    bits |= LEFT[k];
                }
                if lv < hr {
                    bits |= RIGHT[k];
                }
            }
            if bits == 0 {
                continue;
            }
            let fg = match &paint {
                Paint::Height(g) => g.at((h - 1 - cy) as f64 / (h - 1).max(1) as f64),
                Paint::Value(f) => f(vl.max(vr)),
            };
            if let Some(c) = buf.cell_mut((area.x + cx as u16, area.y + cy as u16)) {
                c.set_char(braille(bits)).set_fg(fg);
            }
        }
    }
}

/// One-row braille history, as spans (for embedding in text lines).
pub fn mini_graph(data: &[f64], max: f64, width: usize, grad: &Gradient) -> Vec<Span<'static>> {
    let need = width * 2;
    let max = if max > 0.0 { max } else { 1.0 };
    let lv = |sc: usize| -> (usize, f64) {
        let idx = data.len() as isize - need as isize + sc as isize;
        if idx < 0 {
            return (0, 0.0);
        }
        let v = data[idx as usize];
        (dots_for(v, max, 4), (v / max).clamp(0.0, 1.0))
    };
    (0..width)
        .map(|cx| {
            let ((hl, fl), (hr, fr)) = (lv(cx * 2), lv(cx * 2 + 1));
            let mut bits = 0u8;
            for k in 0..4 {
                let level = 3 - k;
                if level < hl {
                    bits |= LEFT[k];
                }
                if level < hr {
                    bits |= RIGHT[k];
                }
            }
            Span::styled(braille(bits).to_string(), Style::new().fg(grad.at(fl.max(fr))))
        })
        .collect()
}

/// Eight-level block sparkline, as spans.
pub fn sparkline(data: &[f64], max: f64, width: usize, grad: &Gradient, empty: Color) -> Vec<Span<'static>> {
    const B: [char; 9] = [' ', '▁', '▂', '▃', '▄', '▅', '▆', '▇', '█'];
    let max = if max > 0.0 { max } else { 1.0 };
    (0..width)
        .map(|i| {
            let idx = data.len() as isize - width as isize + i as isize;
            if idx < 0 {
                return Span::styled("▁", Style::new().fg(empty));
            }
            let f = (data[idx as usize] / max).clamp(0.0, 1.0);
            let l = (f * 8.0).round() as usize;
            if l == 0 {
                Span::styled("▁", Style::new().fg(empty))
            } else {
                Span::styled(B[l].to_string(), Style::new().fg(grad.at(f)))
            }
        })
        .collect()
}

/// btop-style "■■■■□□□" meter. The gradient runs along the bar, so a full
/// bar ends in the "hot" colour.
pub fn meter(frac: f64, width: usize, grad: &Gradient, empty: Color) -> Vec<Span<'static>> {
    let frac = if frac.is_finite() { frac.clamp(0.0, 1.0) } else { 0.0 };
    let filled = (frac * width as f64).round() as usize;
    let filled = if frac > 0.0 { filled.max(1) } else { 0 };
    let mut v: Vec<Span<'static>> = (0..filled)
        .map(|i| {
            let f = if width > 1 { i as f64 / (width - 1) as f64 } else { 1.0 };
            Span::styled("■", Style::new().fg(grad.at(f)))
        })
        .collect();
    if filled < width {
        v.push(Span::styled("■".repeat(width - filled), Style::new().fg(empty)));
    }
    v
}

/// Write a line into the buffer at (x, y), clipped to `w` columns.
pub fn put(buf: &mut Buffer, x: u16, y: u16, w: u16, line: &Line) {
    buf.set_line(x, y, line, w);
}
