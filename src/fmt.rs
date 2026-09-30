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

/// Truncate to `w` display columns with an ellipsis.
pub fn trunc(s: &str, w: usize) -> String {
    let n = s.chars().count();
    if n <= w {
        s.to_string()
    } else if w == 0 {
        String::new()
    } else {
        let mut t: String = s.chars().take(w - 1).collect();
        t.push('…');
        t
    }
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
    }
}
