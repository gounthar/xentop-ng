//! Runtime binding to libxenstat.
//!
//! The library is dlopen()ed rather than linked so a single binary runs on
//! any Xen release (libxenstat's soname changes every release) and can pick
//! up a locally patched build through LD_LIBRARY_PATH. Symbols added by the
//! xentop-ng libxenstat patches are optional: when absent, the matching
//! columns simply show "-".

use super::fallback::{self, Vbd3Index, XcCpuInfo};
use super::{Avail, DataStatus, Source};
use crate::model::*;
use anyhow::{anyhow, bail, Context, Result};
use libloading::Library;
use std::ffi::{c_char, c_uint, c_ulonglong, c_void, CStr};
use std::time::Instant;

type P = *mut c_void;

const XENSTAT_ALL: c_uint = 0xf;

struct Api {
    init: unsafe extern "C" fn() -> P,
    uninit: unsafe extern "C" fn(P),
    get_node: unsafe extern "C" fn(P, c_uint) -> P,
    free_node: unsafe extern "C" fn(P),

    node_xen_version: unsafe extern "C" fn(P) -> *const c_char,
    node_tot_mem: unsafe extern "C" fn(P) -> c_ulonglong,
    node_free_mem: unsafe extern "C" fn(P) -> c_ulonglong,
    node_num_domains: unsafe extern "C" fn(P) -> c_uint,
    node_num_cpus: unsafe extern "C" fn(P) -> c_uint,
    node_cpu_hz: unsafe extern "C" fn(P) -> c_ulonglong,
    node_domain_by_index: unsafe extern "C" fn(P, c_uint) -> P,

    domain_id: unsafe extern "C" fn(P) -> c_uint,
    domain_name: unsafe extern "C" fn(P) -> *const c_char,
    domain_cpu_ns: unsafe extern "C" fn(P) -> c_ulonglong,
    domain_num_vcpus: unsafe extern "C" fn(P) -> c_uint,
    domain_vcpu: unsafe extern "C" fn(P, c_uint) -> P,
    domain_cur_mem: unsafe extern "C" fn(P) -> c_ulonglong,
    domain_max_mem: unsafe extern "C" fn(P) -> c_ulonglong,
    domain_dying: unsafe extern "C" fn(P) -> c_uint,
    domain_crashed: unsafe extern "C" fn(P) -> c_uint,
    domain_shutdown: unsafe extern "C" fn(P) -> c_uint,
    domain_paused: unsafe extern "C" fn(P) -> c_uint,
    domain_running: unsafe extern "C" fn(P) -> c_uint,
    domain_num_networks: unsafe extern "C" fn(P) -> c_uint,
    domain_network: unsafe extern "C" fn(P, c_uint) -> P,
    domain_num_vbds: unsafe extern "C" fn(P) -> c_uint,
    domain_vbd: unsafe extern "C" fn(P, c_uint) -> P,

    vcpu_online: unsafe extern "C" fn(P) -> c_uint,
    vcpu_ns: unsafe extern "C" fn(P) -> c_ulonglong,

    network_id: unsafe extern "C" fn(P) -> c_uint,
    network_rbytes: unsafe extern "C" fn(P) -> c_ulonglong,
    network_rpackets: unsafe extern "C" fn(P) -> c_ulonglong,
    network_rerrs: unsafe extern "C" fn(P) -> c_ulonglong,
    network_rdrop: unsafe extern "C" fn(P) -> c_ulonglong,
    network_tbytes: unsafe extern "C" fn(P) -> c_ulonglong,
    network_tpackets: unsafe extern "C" fn(P) -> c_ulonglong,
    network_terrs: unsafe extern "C" fn(P) -> c_ulonglong,
    network_tdrop: unsafe extern "C" fn(P) -> c_ulonglong,

    vbd_type: unsafe extern "C" fn(P) -> c_uint,
    vbd_dev: unsafe extern "C" fn(P) -> c_uint,
    vbd_oo_reqs: unsafe extern "C" fn(P) -> c_ulonglong,
    vbd_rd_reqs: unsafe extern "C" fn(P) -> c_ulonglong,
    vbd_wr_reqs: unsafe extern "C" fn(P) -> c_ulonglong,
    vbd_rd_sects: unsafe extern "C" fn(P) -> c_ulonglong,
    vbd_wr_sects: unsafe extern "C" fn(P) -> c_ulonglong,
    vbd_error: Option<unsafe extern "C" fn(P) -> bool>,

    ext: Option<ExtApi>,
    pcpu_idle_ns: Option<unsafe extern "C" fn(P, c_uint) -> c_ulonglong>,
    /// Slots in the pCPU idle array (max_cpu_id + 1); may exceed num_cpus.
    num_pcpu_idle: Option<unsafe extern "C" fn(P) -> c_uint>,
}

/// Symbols from the xentop-ng libxenstat patch.
struct ExtApi {
    vbd_has_ext: unsafe extern "C" fn(P) -> c_uint,
    vbd_rd_reqs_done: unsafe extern "C" fn(P) -> c_ulonglong,
    vbd_wr_reqs_done: unsafe extern "C" fn(P) -> c_ulonglong,
    vbd_rd_usecs: unsafe extern "C" fn(P) -> c_ulonglong,
    vbd_wr_usecs: unsafe extern "C" fn(P) -> c_ulonglong,
    vbd_io_errors: unsafe extern "C" fn(P) -> c_ulonglong,
}

pub struct XenstatSource {
    api: Api,
    handle: P,
    lib_name: String,
    hostname: String,
    /// pCPU idle fallback, opened when libxenstat lacks pcpu_idle_ns.
    xc: Option<XcCpuInfo>,
    status: DataStatus,
    // Keeps the function pointers above valid; declared last so it is
    // dropped after `handle` has been released in Drop.
    _lib: Library,
}

fn candidates() -> Vec<String> {
    let mut v = Vec::new();
    // Versioned names first so LD_LIBRARY_PATH overrides (which usually only
    // ship the versioned file) win over the system development symlink.
    for minor in (10..=40).rev() {
        v.push(format!("libxenstat.so.4.{minor}"));
    }
    v.push("libxenstat.so".into());
    v
}

impl XenstatSource {
    pub fn open(explicit: Option<&str>) -> Result<Self> {
        let names = match explicit {
            Some(p) => {
                check_lib_path(p)?;
                vec![p.to_string()]
            }
            None => candidates(),
        };
        let mut last_err = None;
        let (lib, lib_name) = names
            .into_iter()
            .find_map(|n| match unsafe { Library::new(&n) } {
                Ok(l) => Some((l, n)),
                Err(e) => {
                    last_err = Some(e);
                    None
                }
            })
            .ok_or_else(|| {
                anyhow!(
                    "could not load libxenstat ({}); is this a Xen dom0? Try --demo",
                    last_err.map(|e| e.to_string()).unwrap_or_default()
                )
            })?;

        macro_rules! req {
            ($name:literal) => {
                *unsafe { lib.get(concat!($name, "\0").as_bytes()) }
                    .with_context(|| format!("{lib_name}: missing symbol {}", $name))?
            };
        }
        macro_rules! opt {
            ($name:literal) => {
                unsafe { lib.get(concat!($name, "\0").as_bytes()) }
                    .ok()
                    .map(|s| *s)
            };
        }

        let ext = (|| {
            Some(ExtApi {
                vbd_has_ext: opt!("xenstat_vbd_has_ext")?,
                vbd_rd_reqs_done: opt!("xenstat_vbd_rd_reqs_done")?,
                vbd_wr_reqs_done: opt!("xenstat_vbd_wr_reqs_done")?,
                vbd_rd_usecs: opt!("xenstat_vbd_rd_usecs")?,
                vbd_wr_usecs: opt!("xenstat_vbd_wr_usecs")?,
                vbd_io_errors: opt!("xenstat_vbd_io_errors")?,
            })
        })();

        let api = Api {
            init: req!("xenstat_init"),
            uninit: req!("xenstat_uninit"),
            get_node: req!("xenstat_get_node"),
            free_node: req!("xenstat_free_node"),
            node_xen_version: req!("xenstat_node_xen_version"),
            node_tot_mem: req!("xenstat_node_tot_mem"),
            node_free_mem: req!("xenstat_node_free_mem"),
            node_num_domains: req!("xenstat_node_num_domains"),
            node_num_cpus: req!("xenstat_node_num_cpus"),
            node_cpu_hz: req!("xenstat_node_cpu_hz"),
            node_domain_by_index: req!("xenstat_node_domain_by_index"),
            domain_id: req!("xenstat_domain_id"),
            domain_name: req!("xenstat_domain_name"),
            domain_cpu_ns: req!("xenstat_domain_cpu_ns"),
            domain_num_vcpus: req!("xenstat_domain_num_vcpus"),
            domain_vcpu: req!("xenstat_domain_vcpu"),
            domain_cur_mem: req!("xenstat_domain_cur_mem"),
            domain_max_mem: req!("xenstat_domain_max_mem"),
            domain_dying: req!("xenstat_domain_dying"),
            domain_crashed: req!("xenstat_domain_crashed"),
            domain_shutdown: req!("xenstat_domain_shutdown"),
            domain_paused: req!("xenstat_domain_paused"),
            domain_running: req!("xenstat_domain_running"),
            domain_num_networks: req!("xenstat_domain_num_networks"),
            domain_network: req!("xenstat_domain_network"),
            domain_num_vbds: req!("xenstat_domain_num_vbds"),
            domain_vbd: req!("xenstat_domain_vbd"),
            vcpu_online: req!("xenstat_vcpu_online"),
            vcpu_ns: req!("xenstat_vcpu_ns"),
            network_id: req!("xenstat_network_id"),
            network_rbytes: req!("xenstat_network_rbytes"),
            network_rpackets: req!("xenstat_network_rpackets"),
            network_rerrs: req!("xenstat_network_rerrs"),
            network_rdrop: req!("xenstat_network_rdrop"),
            network_tbytes: req!("xenstat_network_tbytes"),
            network_tpackets: req!("xenstat_network_tpackets"),
            network_terrs: req!("xenstat_network_terrs"),
            network_tdrop: req!("xenstat_network_tdrop"),
            vbd_type: req!("xenstat_vbd_type"),
            vbd_dev: req!("xenstat_vbd_dev"),
            vbd_oo_reqs: req!("xenstat_vbd_oo_reqs"),
            vbd_rd_reqs: req!("xenstat_vbd_rd_reqs"),
            vbd_wr_reqs: req!("xenstat_vbd_wr_reqs"),
            vbd_rd_sects: req!("xenstat_vbd_rd_sects"),
            vbd_wr_sects: req!("xenstat_vbd_wr_sects"),
            vbd_error: opt!("xenstat_vbd_error"),
            ext,
            pcpu_idle_ns: opt!("xenstat_node_pcpu_idle_ns"),
            num_pcpu_idle: opt!("xenstat_node_num_pcpu_idle"),
        };

        let handle = unsafe { (api.init)() };
        if handle.is_null() {
            bail!("xenstat_init() failed: are you root in dom0 (need /dev/xen/privcmd and xenstore)?");
        }
        let hostname = std::fs::read_to_string("/proc/sys/kernel/hostname")
            .map(|s| s.trim().to_string())
            .unwrap_or_else(|_| "xen".into());

        let xc = if api.pcpu_idle_ns.is_none() {
            XcCpuInfo::open()
        } else {
            None
        };
        Ok(Self {
            api,
            handle,
            lib_name,
            hostname,
            xc,
            status: DataStatus::default(),
            _lib: lib,
        })
    }
}

/// `--lib` loads code into a root process. If xentop-ng is ever granted to
/// someone through sudo, that must not become "run any .so as root": as
/// root, only accept an absolute path to a root-owned file whose directories
/// are all root-owned and not group/world-writable.
fn check_lib_path(p: &str) -> Result<()> {
    use std::os::unix::fs::MetadataExt;
    if unsafe { libc::geteuid() } != 0 {
        return Ok(());
    }
    let path = std::path::Path::new(p);
    if !path.is_absolute() {
        bail!("--lib needs an absolute path when running as root");
    }
    let real = std::fs::canonicalize(path).with_context(|| format!("--lib {p}"))?;
    for a in real.ancestors() {
        let m = std::fs::metadata(a).with_context(|| format!("--lib: {}", a.display()))?;
        if m.uid() != 0 || m.mode() & 0o022 != 0 {
            bail!(
                "--lib: refusing {}: {} must be owned by root and not group/world-writable",
                p,
                a.display()
            );
        }
    }
    Ok(())
}

fn cstr(p: *const c_char) -> String {
    if p.is_null() {
        return String::new();
    }
    crate::fmt::sanitize(&unsafe { CStr::from_ptr(p) }.to_string_lossy())
}

impl Source for XenstatSource {
    fn sample(&mut self) -> Result<Snapshot> {
        let a = &self.api;
        let node = unsafe { (a.get_node)(self.handle, XENSTAT_ALL) };
        if node.is_null() {
            bail!("xenstat_get_node() failed");
        }
        let at = Instant::now();

        // SAFETY: every pointer below is owned by `node` and stays valid
        // until xenstat_free_node(), which we call once we've copied out.
        let snap = unsafe {
            let num_cpus = (a.node_num_cpus)(node);
            let slots = a.num_pcpu_idle.map(|f| f(node)).unwrap_or(num_cpus);
            let pcpu_idle_ns = a
                .pcpu_idle_ns
                // Offline CPUs (and holes left by smt=0) report 0.
                .map(|f| {
                    (0..slots)
                        .map(|c| (c, f(node, c)))
                        .filter(|&(_, ns)| ns != 0)
                        .collect::<Vec<_>>()
                })
                .filter(|v| !v.is_empty());

            let n = (a.node_num_domains)(node);
            let mut domains = Vec::with_capacity(n as usize);
            for i in 0..n {
                let dp = (a.node_domain_by_index)(node, i);
                if dp.is_null() {
                    continue;
                }
                let state = if (a.domain_crashed)(dp) != 0 {
                    DomState::Crashed
                } else if (a.domain_dying)(dp) != 0 {
                    DomState::Dying
                } else if (a.domain_shutdown)(dp) != 0 {
                    DomState::Shutdown
                } else if (a.domain_paused)(dp) != 0 {
                    DomState::Paused
                } else if (a.domain_running)(dp) != 0 {
                    DomState::Running
                } else {
                    DomState::Blocked
                };

                let vcpus = (0..(a.domain_num_vcpus)(dp))
                    .filter_map(|j| {
                        let v = (a.domain_vcpu)(dp, j);
                        (!v.is_null()).then(|| VcpuRaw {
                            online: (a.vcpu_online)(v) != 0,
                            ns: (a.vcpu_ns)(v),
                        })
                    })
                    .collect();

                let nets = (0..(a.domain_num_networks)(dp))
                    .filter_map(|j| {
                        let x = (a.domain_network)(dp, j);
                        (!x.is_null()).then(|| NetRaw {
                            id: (a.network_id)(x),
                            rbytes: (a.network_rbytes)(x),
                            rpackets: (a.network_rpackets)(x),
                            rerrs: (a.network_rerrs)(x),
                            rdrop: (a.network_rdrop)(x),
                            tbytes: (a.network_tbytes)(x),
                            tpackets: (a.network_tpackets)(x),
                            terrs: (a.network_terrs)(x),
                            tdrop: (a.network_tdrop)(x),
                        })
                    })
                    .collect();

                let vbds = (0..(a.domain_num_vbds)(dp))
                    .filter_map(|j| {
                        let x = (a.domain_vbd)(dp, j);
                        if x.is_null() {
                            return None;
                        }
                        let ext = a.ext.as_ref().and_then(|e| {
                            ((e.vbd_has_ext)(x) != 0).then(|| VbdExt {
                                rd_done: (e.vbd_rd_reqs_done)(x),
                                wr_done: (e.vbd_wr_reqs_done)(x),
                                rd_usecs: (e.vbd_rd_usecs)(x),
                                wr_usecs: (e.vbd_wr_usecs)(x),
                                io_errors: (e.vbd_io_errors)(x),
                            })
                        });
                        Some(VbdRaw {
                            dev: (a.vbd_dev)(x),
                            kind: VbdKind::from_xenstat((a.vbd_type)(x)),
                            oo_reqs: (a.vbd_oo_reqs)(x),
                            rd_reqs: (a.vbd_rd_reqs)(x),
                            wr_reqs: (a.vbd_wr_reqs)(x),
                            rd_sects: (a.vbd_rd_sects)(x),
                            wr_sects: (a.vbd_wr_sects)(x),
                            error: a.vbd_error.map(|f| f(x)).unwrap_or(false),
                            ext,
                        })
                    })
                    .collect();

                domains.push(DomainRaw {
                    id: (a.domain_id)(dp),
                    name: cstr((a.domain_name)(dp)),
                    state,
                    cpu_ns: (a.domain_cpu_ns)(dp),
                    vcpus,
                    cur_mem: (a.domain_cur_mem)(dp),
                    max_mem: (a.domain_max_mem)(dp),
                    nets,
                    vbds,
                });
            }

            Snapshot {
                at,
                hostname: self.hostname.clone(),
                xen_version: cstr((a.node_xen_version)(node)),
                num_cpus,
                cpu_hz: (a.node_cpu_hz)(node),
                tot_mem: (a.node_tot_mem)(node),
                free_mem: (a.node_free_mem)(node),
                pcpu_idle_ns,
                domains,
            }
        };
        unsafe { (a.free_node)(node) };
        let mut snap = snap;
        self.fill_gaps(&mut snap);
        Ok(snap)
    }

    fn describe(&self) -> String {
        self.lib_name.clone()
    }

    fn status(&self) -> DataStatus {
        self.status.clone()
    }
}

impl XenstatSource {
    /// Patch over what this libxenstat lacks, and record where each class of
    /// metrics came from.
    fn fill_gaps(&mut self, snap: &mut Snapshot) {
        // pCPU idle time.
        self.status.pcpu = if snap.pcpu_idle_ns.is_some() {
            Avail::Lib
        } else if let Some(v) = self.xc.as_mut().and_then(|xc| xc.idle()) {
            snap.pcpu_idle_ns = Some(v);
            Avail::Fallback
        } else {
            Avail::Missing
        };

        // VIFs: stock libxenstat drops them all on hosts without a Linux
        // bridge. Rebuild any guest's VIFs from /proc/net/dev.
        self.status.vifs = Avail::Lib;
        if snap.domains.iter().any(|d| d.id != 0 && d.nets.is_empty()) {
            let vifs = fallback::proc_net_vifs();
            for d in snap.domains.iter_mut().filter(|d| d.id != 0 && d.nets.is_empty()) {
                let mut nets: Vec<NetRaw> = vifs
                    .iter()
                    .filter(|((domid, _), _)| *domid == d.id)
                    .map(|(_, n)| *n)
                    .collect();
                if !nets.is_empty() {
                    nets.sort_by_key(|n| n.id);
                    d.nets = nets;
                    self.status.vifs = Avail::Fallback;
                }
            }
        }

        // tapdisk3 latency.
        let vbd3 = |snap: &Snapshot| {
            snap.domains
                .iter()
                .flat_map(|d| d.vbds.iter())
                .filter(|v| v.kind == VbdKind::Vbd3)
                .count()
        };
        let total = vbd3(snap);
        self.status.vbd_latency = if total == 0 {
            Avail::NotApplicable
        } else if self.api.ext.is_some() {
            let with_ext = snap
                .domains
                .iter()
                .flat_map(|d| d.vbds.iter())
                .filter(|v| v.ext.is_some())
                .count();
            if with_ext > 0 {
                Avail::Lib
            } else {
                Avail::Missing
            }
        } else {
            let idx = Vbd3Index::scan();
            let mut found = 0;
            if !idx.is_empty() {
                for d in &mut snap.domains {
                    for v in d.vbds.iter_mut().filter(|v| v.kind == VbdKind::Vbd3) {
                        v.ext = idx.read(d.id, v.dev);
                        found += v.ext.is_some() as usize;
                    }
                }
            }
            if found > 0 {
                Avail::Fallback
            } else {
                Avail::Missing
            }
        };
    }
}

impl Drop for XenstatSource {
    fn drop(&mut self) {
        unsafe { (self.api.uninit)(self.handle) };
    }
}
