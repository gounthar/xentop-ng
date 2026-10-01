//! Collectors used when the loaded libxenstat lacks what we need, until the
//! libxenstat patches (see libxenstat/) are available upstream:
//!
//! - per-pCPU idle time via libxenctrl's xc_getcpuinfo();
//! - per-domain steal time via XCP-ng's libxenctrl
//!   xc_get_runstate_info_ext() (whole-domain runstate, XenServer patch
//!   queue), when the hypervisor lacks XEN_DOMCTL_get_vcpu_runstate;
//! - VIF counters from /proc/net/dev (stock libxenstat loses every VIF on
//!   hosts without a Linux bridge, e.g. Open vSwitch);
//! - tapdisk3 service-time counters from its shared-memory stats files.

use crate::model::{NetRaw, VbdExt};
use libloading::Library;
use std::collections::HashMap;
use std::ffi::CString;
use std::ffi::{c_int, c_uint, c_void};
use std::fs::File;
use std::io::Read;
use std::os::fd::{AsRawFd, FromRawFd, OwnedFd};

// ---------------------------------------------------------------------------
// pCPU idle time

/// Upper bound on physical CPU ids we ask the hypervisor about.
const MAX_PCPUS: usize = 8192;

/// Minimal libxenctrl binding: just enough for xc_getcpuinfo(). Its
/// signature and `xc_cpuinfo_t` (one uint64 idletime) have not changed in
/// well over a decade, unlike most of libxenctrl.
pub struct XcCpuInfo {
    xch: *mut c_void,
    getcpuinfo: unsafe extern "C" fn(*mut c_void, c_int, *mut u64, *mut c_int) -> c_int,
    close: unsafe extern "C" fn(*mut c_void) -> c_int,
    buf: Vec<u64>,
    _lib: Library,
}

type XcOpen = unsafe extern "C" fn(*mut c_void, *mut c_void, c_uint) -> *mut c_void;

fn libxenctrl() -> Option<Library> {
    let names: Vec<String> = (10..=40)
        .rev()
        .map(|m| format!("libxenctrl.so.4.{m}"))
        .chain(["libxenctrl.so".to_string()])
        .collect();
    // Prefer the libxenctrl that libxenstat itself pulled in, so both
    // talk to the hypervisor through the same version.
    let loaded = names.iter().find_map(|n| {
        unsafe { libloading::os::unix::Library::open(Some(n), libc::RTLD_NOW | libc::RTLD_NOLOAD) }
            .ok()
            .map(Library::from)
    });
    loaded.or_else(|| names.iter().find_map(|n| unsafe { Library::new(n) }.ok()))
}

impl XcCpuInfo {
    pub fn open() -> Option<Self> {
        let lib = libxenctrl()?;
        unsafe {
            let open: XcOpen = *lib.get(b"xc_interface_open\0").ok()?;
            let getcpuinfo = *lib.get(b"xc_getcpuinfo\0").ok()?;
            let close = *lib.get(b"xc_interface_close\0").ok()?;
            let xch = open(std::ptr::null_mut(), std::ptr::null_mut(), 0);
            if xch.is_null() {
                return None;
            }
            Some(XcCpuInfo {
                xch,
                getcpuinfo,
                close,
                buf: vec![0; MAX_PCPUS],
                _lib: lib,
            })
        }
    }

    /// (cpu id, cumulative idle ns) for every online pCPU.
    pub fn idle(&mut self) -> Option<Vec<(u32, u64)>> {
        let mut n: c_int = 0;
        // SAFETY: buf holds MAX_PCPUS xc_cpuinfo_t (a single u64 each) and
        // the hypervisor writes at most max_cpus entries.
        let rc = unsafe { (self.getcpuinfo)(self.xch, MAX_PCPUS as c_int, self.buf.as_mut_ptr(), &mut n) };
        if rc != 0 || n <= 0 {
            return None;
        }
        let n = (n as usize).min(MAX_PCPUS);
        let v: Vec<(u32, u64)> = self.buf[..n]
            .iter()
            .enumerate()
            .filter(|(_, &ns)| ns != 0) // offline CPUs report 0
            .map(|(i, &ns)| (i as u32, ns))
            .collect();
        (!v.is_empty()).then_some(v)
    }
}

impl Drop for XcCpuInfo {
    fn drop(&mut self) {
        unsafe { (self.close)(self.xch) };
    }
}

// ---------------------------------------------------------------------------
// Domain steal time (XCP-ng / XenServer hypervisors)

/// XEN_DOMCTL_get_runstate_info, from XenServer's patch queue (shipped by
/// XCP-ng and XenServer, not upstream).
const DOMCTL_GET_RUNSTATE_INFO: u32 = 98;
/// Room for a `struct xen_domctl` (144 bytes on x86_64) with margin.
const DOMCTL_WORDS: usize = 64;
/// u64 index of `u.domain_runstate.runnable` in `struct xen_domctl`:
/// 16 bytes of header, then state, missed_changes (u32 each),
/// state_entry_time and time[6] (u64 each) before it.
const RUNNABLE_WORD: usize = (16 + 8 + 8 + 6 * 8) / 8;

/// Whole-domain runstate via XCP-ng's libxenctrl, which has
/// `xc_get_runstate_info_ext()`. It reports the domain's runnable time as
/// the average over its vCPUs, so per-vCPU detail is not available.
pub struct XcDomRunstate {
    xch: *mut c_void,
    get: unsafe extern "C" fn(*mut c_void, u32, *mut c_void) -> c_int,
    close: unsafe extern "C" fn(*mut c_void) -> c_int,
    buf: Box<[u64; DOMCTL_WORDS]>,
    _lib: Library,
}

impl XcDomRunstate {
    pub fn open() -> Option<Self> {
        if !cfg!(all(target_arch = "x86_64", target_endian = "little")) {
            return None;
        }
        let lib = libxenctrl()?;
        unsafe {
            let open: XcOpen = *lib.get(b"xc_interface_open\0").ok()?;
            let get = *lib.get(b"xc_get_runstate_info_ext\0").ok()?;
            let close = *lib.get(b"xc_interface_close\0").ok()?;
            let xch = open(std::ptr::null_mut(), std::ptr::null_mut(), 0);
            if xch.is_null() {
                return None;
            }
            Some(XcDomRunstate {
                xch,
                get,
                close,
                buf: Box::new([0; DOMCTL_WORDS]),
                _lib: lib,
            })
        }
    }

    /// Runnable time of `domid` summed over its `nr_vcpus` vCPUs (ns).
    pub fn runnable_ns(&mut self, domid: u32, nr_vcpus: usize) -> Option<u64> {
        self.buf.fill(0);
        // SAFETY: buf is larger than struct xen_domctl and 8-byte aligned;
        // the call writes at most one struct xen_domctl into it.
        let rc = unsafe { (self.get)(self.xch, domid, self.buf.as_mut_ptr().cast()) };
        parse_dom_runstate(rc, &self.buf, domid, nr_vcpus)
    }
}

/// Check the returned domctl really is ours (cmd and domain echoed back)
/// before trusting the offsets, then undo the per-vCPU averaging.
fn parse_dom_runstate(rc: c_int, buf: &[u64; DOMCTL_WORDS], domid: u32, nr_vcpus: usize) -> Option<u64> {
    let cmd = buf[0] as u32;
    let dom = buf[1] as u16;
    if rc != 0 || cmd != DOMCTL_GET_RUNSTATE_INFO || dom as u32 != domid {
        return None;
    }
    Some(buf[RUNNABLE_WORD].saturating_mul(nr_vcpus as u64))
}

impl Drop for XcDomRunstate {
    fn drop(&mut self) {
        unsafe { (self.close)(self.xch) };
    }
}

// ---------------------------------------------------------------------------
// VIFs

/// VIF counters by (domid, devid), parsed from /proc/net/dev. Counters are
/// from dom0's side, like libxenstat's. Read through PID 1 so we see the
/// initial network namespace (where VIFs live) whatever ours is.
pub fn proc_net_vifs() -> HashMap<(u32, u32), NetRaw> {
    for path in ["/proc/1/net/dev", "/proc/net/dev"] {
        if let Ok(f) = File::open(path) {
            let mut s = String::new();
            // Bounded: a few hundred bytes per interface.
            if f.take(4 << 20).read_to_string(&mut s).is_ok() {
                return parse_proc_net_dev(&s);
            }
        }
    }
    HashMap::new()
}

fn parse_proc_net_dev(s: &str) -> HashMap<(u32, u32), NetRaw> {
    let mut out = HashMap::new();
    for line in s.lines().skip(2) {
        let Some((iface, rest)) = line.split_once(':') else {
            continue;
        };
        let Some((dom, dev)) = iface.trim().strip_prefix("vif").and_then(|v| v.split_once('.')) else {
            continue;
        };
        let (Ok(domid), Ok(devid)) = (dom.parse::<u32>(), dev.parse::<u32>()) else {
            continue;
        };
        let f: Vec<u64> = rest.split_whitespace().map(|x| x.parse().unwrap_or(0)).collect();
        if f.len() < 12 {
            continue;
        }
        out.insert(
            (domid, devid),
            NetRaw {
                id: devid,
                network: None,
                rbytes: f[0],
                rpackets: f[1],
                rerrs: f[2],
                rdrop: f[3],
                tbytes: f[8],
                tpackets: f[9],
                terrs: f[10],
                tdrop: f[11],
            },
        );
    }
    out
}

// ---------------------------------------------------------------------------
// tapdisk3 stats

/// tapdisk3's shared-memory per-VBD stats, version 1 (blktap
/// drivers/tapdisk-metrics-stats.h): u32 version, u32 pad, then 11 u64.
const VBD3_STATS_LEN: usize = 96;

/// Most td3-* directories we look at per sample; real hosts have one per
/// tapdisk, i.e. per attached VDI.
const MAX_TD3_DIRS: usize = 4096;

/// tapdisk3 stats, keyed by (domid, vbd dev), collected once per sample.
///
/// /dev/shm is world-writable, so everything there is treated as hostile:
/// directories and files are opened relative to already-verified parent
/// file descriptors, never through symlinks, and must be root-owned and not
/// writable by anyone else. Names are parsed strictly, and a tapdisk's
/// directory only counts if that PID is alive.
#[derive(Default)]
pub struct Vbd3Index {
    stats: HashMap<(u32, u32), VbdExt>,
}

/// `fstat` an fd we own and check it is root-owned, not group/other
/// writable, and of the expected type.
fn trusted_fd(fd: &OwnedFd, want: libc::mode_t) -> Option<libc::stat> {
    let mut st: libc::stat = unsafe { std::mem::zeroed() };
    // SAFETY: valid fd and a properly sized stat buffer.
    if unsafe { libc::fstat(fd.as_raw_fd(), &mut st) } != 0 {
        return None;
    }
    let ok = st.st_uid == 0 && st.st_mode & 0o022 == 0 && st.st_mode & libc::S_IFMT == want;
    ok.then_some(st)
}

fn openat(dir: &OwnedFd, name: &str, flags: libc::c_int) -> Option<OwnedFd> {
    let c = CString::new(name).ok()?;
    let flags = flags | libc::O_NOFOLLOW | libc::O_CLOEXEC | libc::O_NONBLOCK;
    // SAFETY: valid dir fd and NUL-terminated name; we own the returned fd.
    let fd = unsafe { libc::openat(dir.as_raw_fd(), c.as_ptr(), flags) };
    (fd >= 0).then(|| unsafe { OwnedFd::from_raw_fd(fd) })
}

/// "td3-<pid>" -> pid, digits only.
fn td3_pid(name: &str) -> Option<u32> {
    let n = name.strip_prefix("td3-")?;
    (!n.is_empty() && n.bytes().all(|b| b.is_ascii_digit())).then(|| n.parse().ok())?
}

/// "vbd-<domid>-<dev>" -> (domid, dev), digits only.
fn vbd_key(name: &str) -> Option<(u32, u32)> {
    let (d, v) = name.strip_prefix("vbd-")?.split_once('-')?;
    let num = |x: &str| (!x.is_empty() && x.bytes().all(|b| b.is_ascii_digit())).then(|| x.parse().ok())?;
    Some((num(d)?, num(v)?))
}

impl Vbd3Index {
    pub fn scan() -> Self {
        let mut stats = HashMap::new();
        let Ok(shm_dir) = File::open("/dev/shm") else {
            return Self { stats };
        };
        let shm = OwnedFd::from(shm_dir);
        let Ok(entries) = std::fs::read_dir("/dev/shm") else {
            return Self { stats };
        };
        for e in entries.flatten().take(MAX_TD3_DIRS * 4) {
            let name = e.file_name();
            let Some(name) = name.to_str() else { continue };
            let Some(pid) = td3_pid(name) else { continue };
            // Stale directories of dead tapdisks linger; skip them.
            if pid == 0 || unsafe { libc::kill(pid as libc::pid_t, 0) } != 0 {
                continue;
            }
            let Some(dir) = openat(&shm, name, libc::O_RDONLY | libc::O_DIRECTORY) else {
                continue;
            };
            if trusted_fd(&dir, libc::S_IFDIR).is_none() {
                continue;
            }
            // List through the verified fd (/proc/self/fd/N), not the path.
            let Ok(files) = std::fs::read_dir(format!("/proc/self/fd/{}", dir.as_raw_fd())) else {
                continue;
            };
            for f in files.flatten().take(MAX_TD3_DIRS) {
                let fname = f.file_name();
                let Some(fname) = fname.to_str() else { continue };
                let Some(key) = vbd_key(fname) else { continue };
                if let Some(ext) = read_stats(&dir, fname) {
                    stats.insert(key, ext);
                }
            }
            if stats.len() >= MAX_TD3_DIRS {
                break;
            }
        }
        Self { stats }
    }

    pub fn is_empty(&self) -> bool {
        self.stats.is_empty()
    }

    pub fn read(&self, domid: u32, dev: u32) -> Option<VbdExt> {
        self.stats.get(&(domid, dev)).copied()
    }
}

fn read_stats(dir: &OwnedFd, name: &str) -> Option<VbdExt> {
    let fd = openat(dir, name, libc::O_RDONLY)?;
    let st = trusted_fd(&fd, libc::S_IFREG)?;
    if st.st_nlink != 1 || !(VBD3_STATS_LEN as libc::off_t..=1 << 16).contains(&st.st_size) {
        return None;
    }
    let mut b = [0u8; VBD3_STATS_LEN];
    File::from(fd).read_exact(&mut b).ok()?;
    parse_vbd3(&b)
}

fn parse_vbd3(b: &[u8; VBD3_STATS_LEN]) -> Option<VbdExt> {
    let u32_at = |o: usize| u32::from_ne_bytes(b[o..o + 4].try_into().unwrap());
    let u64_at = |i: usize| u64::from_ne_bytes(b[8 + i * 8..16 + i * 8].try_into().unwrap());
    if u32_at(0) != 1 {
        return None;
    }
    // 0 oo_reqs, 1 rd submitted, 2 rd completed, 3 rd sectors, 4 rd ticks,
    // 5 wr submitted, 6 wr completed, 7 wr sectors, 8 wr ticks, 9 io_errors
    Some(VbdExt {
        rd_done: u64_at(2),
        wr_done: u64_at(6),
        rd_usecs: u64_at(4),
        wr_usecs: u64_at(8),
        io_errors: u64_at(9),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn proc_net_dev() {
        let s = "Inter-|   Receive                                                |  Transmit
 face |bytes    packets errs drop fifo frame compressed multicast|bytes    packets errs drop fifo colls carrier compressed
  eth0: 100 2 0 0 0 0 0 0 200 3 0 0 0 0 0 0
vif15.0: 142505491  995684    1    2    0     0          0         0 5000 60 3 4 0 0 0 0
vif3.1: 1 2 3 4 0 0 0 0 5 6 7 8 0 0 0 0
vifbogus: 1 2 3 4 0 0 0 0 5 6 7 8 0 0 0 0
";
        let m = parse_proc_net_dev(s);
        assert_eq!(m.len(), 2);
        let v = &m[&(15, 0)];
        assert_eq!(
            (v.rbytes, v.rpackets, v.rerrs, v.rdrop),
            (142505491, 995684, 1, 2)
        );
        assert_eq!((v.tbytes, v.tpackets, v.terrs, v.tdrop), (5000, 60, 3, 4));
        assert_eq!(m[&(3, 1)].id, 1);
    }

    #[test]
    fn strict_names() {
        assert_eq!(td3_pid("td3-1234"), Some(1234));
        for bad in ["td3-", "td3-self", "td3-12a", "td3--1", "td3-1/..", "xtd3-1"] {
            assert_eq!(td3_pid(bad), None, "{bad}");
        }
        assert_eq!(vbd_key("vbd-5-51712"), Some((5, 51712)));
        for bad in [
            "vbd-5",
            "vbd--1-2",
            "vbd-5-",
            "vbd-5-1-2",
            "vbd-a-1",
            "vbd-5-768.tmp",
        ] {
            assert_eq!(vbd_key(bad), None, "{bad}");
        }
    }

    #[test]
    fn dom_runstate_layout() {
        let mut b = [0u64; DOMCTL_WORDS];
        b[0] = DOMCTL_GET_RUNSTATE_INFO as u64 | (0x15 << 32); // cmd, interface_version
        b[1] = 7; // domain
        b[RUNNABLE_WORD] = 1_000;
        assert_eq!(RUNNABLE_WORD, 10);
        assert_eq!(parse_dom_runstate(0, &b, 7, 4), Some(4_000));
        assert_eq!(parse_dom_runstate(-1, &b, 7, 4), None);
        assert_eq!(parse_dom_runstate(0, &b, 8, 4), None);
        b[0] = 99;
        assert_eq!(parse_dom_runstate(0, &b, 7, 4), None);
    }

    #[test]
    fn vbd3_layout() {
        let mut b = [0u8; VBD3_STATS_LEN];
        b[0..4].copy_from_slice(&1u32.to_ne_bytes());
        for i in 0..11u64 {
            let o = 8 + i as usize * 8;
            b[o..o + 8].copy_from_slice(&(100 + i).to_ne_bytes());
        }
        let e = parse_vbd3(&b).unwrap();
        assert_eq!(
            (e.rd_done, e.rd_usecs, e.wr_done, e.wr_usecs, e.io_errors),
            (102, 104, 106, 108, 109)
        );
        b[0..4].copy_from_slice(&2u32.to_ne_bytes());
        assert!(parse_vbd3(&b).is_none());
    }
}
