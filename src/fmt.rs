//! Compact human-readable number formatting.

/// Bytes with binary prefixes: "512B", "3.4K", "12.0M", "1.21G".
pub fn bytes(v: f64) -> String {
    const U: [&str; 6] = ["B", "K", "M", "G", "T", "P"];
    let mut v = v.max(0.0);
    let mut i = 0;
    while v >= 1000.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{v:.0}{}", U[i])
    } else if v < 10.0 {
        format!("{v:.2}{}", U[i])
    } else if v < 100.0 {
        format!("{v:.1}{}", U[i])
    } else {
        format!("{v:.0}{}", U[i])
    }
}

/// Byte rate, blank-ish when idle.
pub fn rate(v: f64) -> String {
    if v < 0.5 {
        "0".into()
    } else {
        bytes(v)
    }
}

/// Counts with SI prefixes: "12", "3.4k", "1.2M".
pub fn count(v: f64) -> String {
    let v = v.max(0.0);
    if v < 1000.0 {
        if v > 0.0 && v < 10.0 && v.fract() != 0.0 {
            format!("{v:.1}")
        } else {
            format!("{v:.0}")
        }
    } else if v < 1e6 {
        format!("{:.1}k", v / 1e3)
    } else {
        format!("{:.1}M", v / 1e6)
    }
}

/// Latency from microseconds: "85µs", "1.24ms", "2.1s".
pub fn lat(us: Option<f64>) -> String {
    match us {
        None => "-".into(),
        Some(u) if u < 1000.0 => format!("{u:.0}µs"),
        Some(u) if u < 10_000.0 => format!("{:.2}ms", u / 1e3),
        Some(u) if u < 1_000_000.0 => format!("{:.1}ms", u / 1e3),
        Some(u) => format!("{:.1}s", u / 1e6),
    }
}

pub fn pct(v: f64) -> String {
    if v < 10.0 {
        format!("{v:.1}%")
    } else {
        format!("{v:.0}%")
    }
}

/// Display columns, so wide (CJK, emoji) names don't break alignment.
pub fn width(s: &str) -> usize {
    unicode_width::UnicodeWidthStr::width(s)
}

/// Truncate to `w` display columns with an ellipsis.
pub fn trunc(s: &str, w: usize) -> String {
    if width(s) <= w {
        return s.to_string();
    }
    if w == 0 {
        return String::new();
    }
    let mut t = String::new();
    let mut used = 0;
    for c in s.chars() {
        let cw = unicode_width::UnicodeWidthChar::width(c).unwrap_or(0);
        if used + cw > w - 1 {
            break;
        }
        used += cw;
        t.push(c);
    }
    t.push('…');
    t
}

/// Truncate and pad to exactly `w` display columns.
pub fn pad(s: &str, w: usize, right: bool) -> String {
    let t = trunc(s, w);
    let fill = " ".repeat(w.saturating_sub(width(&t)));
    if right {
        fill + &t
    } else {
        t + &fill
    }
}

/// Make an untrusted string (e.g. a VM name, which toolstack users with
/// lower privileges than dom0 root can set) safe for terminals and logs:
/// control characters, invisible formatting and bidi overrides become '?'.
pub fn sanitize(s: &str) -> String {
    s.chars()
        .map(|c| {
            let invisible = matches!(c,
                '\u{00AD}' | '\u{061C}' | '\u{180E}' | '\u{200B}'..='\u{200F}' | '\u{2028}'..='\u{202E}'
                | '\u{2060}'..='\u{2064}' | '\u{2066}'..='\u{2069}' | '\u{FEFF}' | '\u{FFF9}'..='\u{FFFB}');
            if c.is_control() || invisible {
                '?'
            } else {
                c
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats() {
        assert_eq!(bytes(512.0), "512B");
        assert_eq!(bytes(1536.0), "1.50K");
        assert_eq!(bytes(64.0 * 1024.0 * 1024.0 * 1024.0), "64.0G");
        assert_eq!(lat(Some(85.0)), "85µs");
        assert_eq!(lat(Some(1240.0)), "1.24ms");
        assert_eq!(count(3400.0), "3.4k");
        assert_eq!(trunc("abcdef", 4), "abc…");
        // Wide characters count double.
        assert_eq!(trunc("宽字符名", 5), "宽字…");
        assert_eq!(width(&pad("宽字", 6, false)), 6);
        assert_eq!(pad("ab", 4, true), "  ab");
    }

    #[test]
    fn sanitizes_hostile_names() {
        assert_eq!(sanitize("vm\x1b]0;pwned\x07"), "vm?]0;pwned?");
        assert_eq!(sanitize("a\u{9b}b\u{7f}c"), "a?b?c");
        assert_eq!(sanitize("\u{202E}gnp.exe"), "?gnp.exe");
        assert_eq!(sanitize("zero\u{200B}width"), "zero?width");
        assert_eq!(sanitize("Café 宽 ok"), "Café 宽 ok");
    }
}
