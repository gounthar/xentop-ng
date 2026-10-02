//! Loading the Xen C libraries at runtime (libxenstat, libxenstore,
//! libxenctrl): which names to try, in which order, and the safety check on
//! a library path given on the command line.
//!
//! Libraries are found by soname through the dynamic linker, so a patched
//! copy on LD_LIBRARY_PATH wins over the system one. Only `--lib PATH`
//! names a file directly; that path is checked before it is loaded.

use anyhow::{bail, Context, Result};
use libloading::Library;
use std::path::{Path, PathBuf};

/// `<base>.so.4.40` down to `<base>.so.4.10`, then `<base>.so`. Versioned
/// names first, so LD_LIBRARY_PATH overrides (which usually only ship the
/// versioned file) win over the system development symlink.
pub fn versioned(base: &str) -> Vec<String> {
    (10..=40)
        .rev()
        .map(|m| format!("{base}.so.4.{m}"))
        .chain([format!("{base}.so")])
        .collect()
}

/// The first of `names` that loads, with its name; else the last error.
pub fn open_first<S: AsRef<str>>(names: &[S]) -> Result<(Library, String), Option<libloading::Error>> {
    let mut last = None;
    for n in names {
        match unsafe { Library::new(n.as_ref()) } {
            Ok(l) => return Ok((l, n.as_ref().to_string())),
            Err(e) => last = Some(e),
        }
    }
    Err(last)
}

/// The first of `names` already loaded in this process (pulled in by
/// another library), without loading anything new.
pub fn already_loaded<S: AsRef<str>>(names: &[S]) -> Option<Library> {
    names.iter().find_map(|n| {
        unsafe { libloading::os::unix::Library::open(Some(n.as_ref()), libc::RTLD_NOW | libc::RTLD_NOLOAD) }
            .ok()
            .map(Library::from)
    })
}

/// `--lib` loads code into a root process. If xentop-ng is ever granted to
/// someone through sudo, that must not become "run any .so as root": as
/// root, only accept an absolute path to a root-owned file whose directories
/// are all root-owned and not group/world-writable.
///
/// Returns the path to load: as root, the resolved path that was checked,
/// not `p`, which may go through a symlink its owner could swap between
/// the check and the load.
pub fn checked_path(p: &str) -> Result<String> {
    if unsafe { libc::geteuid() } != 0 {
        return Ok(p.to_string());
    }
    let path = Path::new(p);
    if !path.is_absolute() {
        bail!("--lib needs an absolute path when running as root");
    }
    let real = std::fs::canonicalize(path).with_context(|| format!("--lib {p}"))?;
    if let Some(a) = unsafe_ancestor(&real, 0)? {
        bail!(
            "--lib: refusing {}: {} must be owned by root and not group/world-writable",
            p,
            a.display()
        );
    }
    real.into_os_string()
        .into_string()
        .map_err(|_| anyhow::anyhow!("--lib {p}: not a UTF-8 path"))
}

/// The first of `real` and its parent directories not owned by `owner`, or
/// writable by group or others.
fn unsafe_ancestor(real: &Path, owner: u32) -> Result<Option<PathBuf>> {
    use std::os::unix::fs::MetadataExt;
    for a in real.ancestors() {
        let m = std::fs::metadata(a).with_context(|| format!("--lib: {}", a.display()))?;
        if m.uid() != owner || m.mode() & 0o022 != 0 {
            return Ok(Some(a.to_path_buf()));
        }
    }
    Ok(None)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn versioned_names_newest_first_then_plain() {
        let v = versioned("libxenstat");
        assert_eq!(v.first().unwrap(), "libxenstat.so.4.40");
        assert_eq!(v[v.len() - 2], "libxenstat.so.4.10");
        assert_eq!(v.last().unwrap(), "libxenstat.so");
        assert_eq!(v.len(), 32);
    }

    #[test]
    fn open_first_reports_the_last_error() {
        assert!(matches!(
            open_first(&["libnope-xentop-ng-1.so", "libnope-xentop-ng-2.so"]),
            Err(Some(_))
        ));
        assert!(matches!(open_first::<&str>(&[]), Err(None)));
        assert!(already_loaded(&["libnope-xentop-ng-1.so"]).is_none());
    }

    #[test]
    fn ancestors_must_be_owned_and_not_writable() {
        // System directories are root-owned and 0755 on any sane host.
        assert_eq!(unsafe_ancestor(Path::new("/usr"), 0).unwrap(), None);
        // Not owned by root: the file itself is refused.
        let dir = std::env::temp_dir().join(format!("xentop-ng-dl-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let f = dir.join("libfake.so");
        std::fs::write(&f, b"").unwrap();
        let me = unsafe { libc::geteuid() };
        if me != 0 {
            assert_eq!(unsafe_ancestor(&f, 0).unwrap(), Some(f.clone()));
        }
        // World-writable (sticky /tmp): refused even with the right owner.
        let tmp = std::fs::canonicalize("/tmp").unwrap();
        assert_eq!(unsafe_ancestor(&tmp, 0).unwrap(), Some(tmp.clone()));
        // Missing files are an error, not a pass.
        assert!(unsafe_ancestor(&dir.join("missing"), me).is_err());
        std::fs::remove_dir_all(&dir).unwrap();
    }
}
