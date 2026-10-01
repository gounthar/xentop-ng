//! Names from xapi, the XCP-ng/XenServer toolstack: SR and VDI name-labels,
//! SR types, and the network behind each VIF.
//!
//! Optional by design. On a host without xapi's socket (plain Xen) nothing
//! here runs and everything else works as before; on an XCP-ng host the
//! UUIDs xenstore gives us get human names on top.
//!
//! xapi is reached over its local Unix socket, which only root can open and
//! which xapi trusts without a password (as `xe` and SM do in dom0). Only
//! login/logout and read-only getters are called, through JSON-RPC. All of
//! it runs on a background thread so a slow or restarting xapi never stalls
//! sampling: each sample hands over the UUIDs it would like named (when
//! they change) and picks up whatever has been resolved so far. Names are
//! re-read every few minutes to catch renames.
//!
//! Names are set by whoever administers the pool, so they are treated like
//! xenstore values: sanitised and bounded before display.

use super::XapiState;
use crate::model::Snapshot;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::fs::FileTypeExt;
use std::os::unix::net::UnixStream;
use std::path::{Path, PathBuf};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::time::Duration;

/// Where xapi listens in dom0 (current, then older XenServer releases).
const SOCKETS: [&str; 2] = ["/var/lib/xcp/xapi", "/var/xapi/xapi"];
const IO_TIMEOUT: Duration = Duration::from_secs(5);
/// Longest response body we accept.
const MAX_RESPONSE: usize = 8 << 20;
const MAX_HEADERS: usize = 64;
/// Longest name we keep, in characters.
const MAX_NAME: usize = 64;
const MAX_KIND: usize = 16;
/// Bounds on what one round asks xapi for.
const MAX_KEYS: usize = 4096;
const MAX_VIFS: usize = 64;
/// Names are re-read this often, to pick up renames.
const REFRESH: Duration = Duration::from_secs(180);
/// How soon to try again after xapi couldn't be reached.
const RETRY: Duration = Duration::from_secs(15);
/// How long quitting waits for the worker to log out.
const LOGOUT_WAIT: Duration = Duration::from_millis(500);

#[derive(Clone, Debug, PartialEq, Eq)]
struct SrName {
    name: Option<String>,
    kind: Option<String>,
}

/// What the worker has resolved so far.
#[derive(Default)]
struct Names {
    state: XapiState,
    srs: HashMap<String, SrName>,
    vdis: HashMap<String, String>,
    /// VM UUID -> VIF devid -> network name.
    vifs: HashMap<String, BTreeMap<u32, String>>,
}

/// UUIDs the current sample would like named.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
struct Wants {
    srs: BTreeSet<String>,
    vdis: BTreeSet<String>,
    /// VM UUID -> its VIF devids; a change re-reads that VM's VIFs.
    vms: BTreeMap<String, BTreeSet<u32>>,
}

impl Wants {
    fn of(snap: &Snapshot) -> Self {
        let mut w = Wants::default();
        for d in &snap.domains {
            for b in d.vbds.iter().filter_map(|v| v.backing.as_ref()) {
                if let Some(sr) = &b.sr {
                    w.srs.insert(sr.clone());
                }
                if let Some(vdi) = &b.vdi {
                    w.vdis.insert(vdi.clone());
                }
            }
            if let Some(vm) = d.vm_uuid.as_ref().filter(|_| !d.nets.is_empty()) {
                w.vms.insert(vm.clone(), d.nets.iter().map(|n| n.id).collect());
            }
        }
        w
    }
}

/// Handle on the background xapi client.
pub struct Xapi {
    names: Arc<Mutex<Names>>,
    tx: Option<mpsc::Sender<Wants>>,
    /// Signalled by the worker once it has logged out.
    bye: Option<mpsc::Receiver<()>>,
    sent: Option<Wants>,
}

impl Xapi {
    /// A client for this host's xapi, or None when there is no xapi (plain
    /// Xen). Nothing is sent to xapi until the first sample.
    pub fn open() -> Option<Self> {
        let path = SOCKETS.iter().map(Path::new).find(|p| is_socket(p))?;
        Some(Self::with_socket(path.to_path_buf()))
    }

    fn with_socket(path: PathBuf) -> Self {
        let names = Arc::new(Mutex::new(Names {
            state: XapiState::Connecting,
            ..Default::default()
        }));
        let (tx, rx) = mpsc::channel();
        let (bye_tx, bye) = mpsc::channel();
        let worker = Worker {
            rpc: Rpc::new(path),
            names: names.clone(),
            done: Wants::default(),
            networks: HashMap::new(),
        };
        let spawned = std::thread::Builder::new()
            .name("xapi".into())
            .spawn(move || worker.run(rx, bye_tx));
        if let Err(e) = spawned {
            if let Ok(mut n) = names.lock() {
                n.state = XapiState::Failed(format!("no thread: {e}"));
            }
        }
        Xapi {
            names,
            tx: Some(tx),
            bye: Some(bye),
            sent: None,
        }
    }

    pub fn state(&self) -> XapiState {
        self.names.lock().map(|n| n.state.clone()).unwrap_or_default()
    }

    /// Put the names known so far on `snap`, and ask for any new UUIDs.
    pub fn fill(&mut self, snap: &mut Snapshot) {
        let wants = Wants::of(snap);
        if self.sent.as_ref() != Some(&wants) {
            if let Some(tx) = &self.tx {
                let _ = tx.send(wants.clone());
            }
            self.sent = Some(wants);
        }
        let Ok(n) = self.names.lock() else {
            return;
        };
        for d in &mut snap.domains {
            for b in d.vbds.iter_mut().filter_map(|v| v.backing.as_mut()) {
                if let Some(sr) = b.sr.as_ref().and_then(|u| n.srs.get(u)) {
                    b.sr_name = sr.name.clone();
                    // xapi's type is exact ("lvmoiscsi" where we'd guess
                    // "lvm"), so it wins.
                    if sr.kind.is_some() {
                        b.sr_kind = sr.kind.clone();
                    }
                }
                if let Some(name) = b.vdi.as_ref().and_then(|u| n.vdis.get(u)) {
                    b.vdi_name = Some(name.clone());
                }
            }
            if let Some(vifs) = d.vm_uuid.as_ref().and_then(|u| n.vifs.get(u)) {
                for net in &mut d.nets {
                    net.network = vifs.get(&net.id).cloned();
                }
            }
        }
    }
}

impl Drop for Xapi {
    fn drop(&mut self) {
        // Hang up, and give the worker a moment to end its session so
        // xapi doesn't keep it around until it times out.
        self.tx = None;
        if let Some(bye) = self.bye.take() {
            let _ = bye.recv_timeout(LOGOUT_WAIT);
        }
    }
}

fn is_socket(p: &Path) -> bool {
    std::fs::metadata(p).is_ok_and(|m| m.file_type().is_socket())
}

/// Sanitised, bounded copy of a name; None when empty.
fn clean(s: &str, max: usize) -> Option<String> {
    let s: String = crate::fmt::sanitize(s.trim()).chars().take(max).collect();
    let s = s.trim_end().to_string();
    (!s.is_empty()).then_some(s)
}

// ---------------------------------------------------------------------------
// Worker

struct Worker {
    rpc: Rpc,
    names: Arc<Mutex<Names>>,
    /// What has been looked up (found or not) since the last refresh.
    done: Wants,
    /// Network ref -> name, for this refresh.
    networks: HashMap<String, Option<String>>,
}

impl Worker {
    fn run(mut self, rx: mpsc::Receiver<Wants>, bye: mpsc::Sender<()>) {
        let mut wants = Wants::default();
        loop {
            let wait = if self.rpc.session.is_some() {
                REFRESH
            } else {
                RETRY
            };
            match rx.recv_timeout(wait) {
                Ok(w) => {
                    // Only the latest matters.
                    wants = rx.try_iter().last().unwrap_or(w);
                }
                Err(RecvTimeoutError::Timeout) => {
                    self.done = Wants::default();
                    self.networks.clear();
                }
                Err(RecvTimeoutError::Disconnected) => break,
            }
            self.round(&wants);
        }
        self.rpc.logout();
        let _ = bye.send(());
    }

    fn set_state(&self, s: XapiState) {
        if let Ok(mut n) = self.names.lock() {
            n.state = s;
        }
    }

    /// Look up whatever is wanted and not looked up yet, then forget what
    /// is no longer wanted.
    fn round(&mut self, wants: &Wants) {
        let res = self.lookup(wants);
        if let Ok(mut n) = self.names.lock() {
            n.srs.retain(|k, _| wants.srs.contains(k));
            n.vdis.retain(|k, _| wants.vdis.contains(k));
            n.vifs.retain(|k, _| wants.vms.contains_key(k));
        }
        self.done.srs.retain(|k| wants.srs.contains(k));
        self.done.vdis.retain(|k| wants.vdis.contains(k));
        self.done.vms.retain(|k, _| wants.vms.contains_key(k));
        match res {
            Ok(()) if self.rpc.session.is_some() => self.set_state(XapiState::Connected),
            Ok(()) => {}
            Err(e) => {
                self.rpc.session = None;
                self.set_state(XapiState::Failed(e));
            }
        }
    }

    /// Errors returned here mean xapi itself is unusable; a UUID xapi
    /// doesn't know is just left unnamed.
    fn lookup(&mut self, wants: &Wants) -> Result<(), String> {
        let srs: Vec<String> = wants
            .srs
            .iter()
            .filter(|u| !self.done.srs.contains(*u))
            .take(MAX_KEYS)
            .cloned()
            .collect();
        for u in srs {
            if let Some(sr) = skip_unknown(self.sr(&u))? {
                self.names
                    .lock()
                    .map_err(|_| "poisoned")?
                    .srs
                    .insert(u.clone(), sr);
            }
            self.done.srs.insert(u);
        }

        let vdis: Vec<String> = wants
            .vdis
            .iter()
            .filter(|u| !self.done.vdis.contains(*u))
            .take(MAX_KEYS)
            .cloned()
            .collect();
        for u in vdis {
            if let Some(Some(name)) = skip_unknown(self.vdi(&u))? {
                self.names
                    .lock()
                    .map_err(|_| "poisoned")?
                    .vdis
                    .insert(u.clone(), name);
            }
            self.done.vdis.insert(u);
        }

        let vms: Vec<(String, BTreeSet<u32>)> = wants
            .vms
            .iter()
            .filter(|(u, ids)| self.done.vms.get(*u) != Some(ids))
            .take(MAX_KEYS)
            .map(|(u, ids)| (u.clone(), ids.clone()))
            .collect();
        for (u, ids) in vms {
            if let Some(vifs) = skip_unknown(self.vifs(&u))? {
                self.names
                    .lock()
                    .map_err(|_| "poisoned")?
                    .vifs
                    .insert(u.clone(), vifs);
            }
            self.done.vms.insert(u, ids);
        }
        Ok(())
    }

    fn sr(&mut self, uuid: &str) -> Result<SrName, Error> {
        let r = self.rpc.call("SR.get_by_uuid", &[uuid.into()])?;
        let rec = self.rpc.call("SR.get_record", &[r])?;
        let field = |k: &str, max| rec.get(k).and_then(Value::as_str).and_then(|s| clean(s, max));
        Ok(SrName {
            name: field("name_label", MAX_NAME),
            kind: field("type", MAX_KIND),
        })
    }

    fn vdi(&mut self, uuid: &str) -> Result<Option<String>, Error> {
        let r = self.rpc.call("VDI.get_by_uuid", &[uuid.into()])?;
        let name = self.rpc.call("VDI.get_name_label", &[r])?;
        Ok(name.as_str().and_then(|s| clean(s, MAX_NAME)))
    }

    /// The network name behind each of a VM's VIFs, by devid.
    fn vifs(&mut self, vm: &str) -> Result<BTreeMap<u32, String>, Error> {
        let r = self.rpc.call("VM.get_by_uuid", &[vm.into()])?;
        let refs = self.rpc.call("VM.get_VIFs", &[r])?;
        let mut out = BTreeMap::new();
        for vif in refs.as_array().into_iter().flatten().take(MAX_VIFS) {
            let rec = match skip_unknown(self.rpc.call("VIF.get_record", std::slice::from_ref(vif)))
                .map_err(Error::Down)?
            {
                Some(rec) => rec,
                None => continue,
            };
            let devid = rec
                .get("device")
                .and_then(Value::as_str)
                .and_then(|s| s.parse::<u32>().ok());
            let net = rec.get("network").and_then(Value::as_str);
            let (Some(devid), Some(net)) = (devid, net) else {
                continue;
            };
            let name = match self.networks.get(net) {
                Some(n) => n.clone(),
                None => {
                    let n = skip_unknown(self.rpc.call("network.get_name_label", &[net.into()]))
                        .map_err(Error::Down)?
                        .and_then(|v| v.as_str().and_then(|s| clean(s, MAX_NAME)));
                    if self.networks.len() < MAX_KEYS {
                        self.networks.insert(net.to_string(), n.clone());
                    }
                    n
                }
            };
            if let Some(name) = name {
                out.insert(devid, name);
            }
        }
        Ok(out)
    }
}

/// An API error about one object (unknown UUID, object just deleted...)
/// becomes None; xapi being unreachable stays an error.
fn skip_unknown<T>(r: Result<T, Error>) -> Result<Option<T>, String> {
    match r {
        Ok(v) => Ok(Some(v)),
        Err(Error::Api(_)) => Ok(None),
        Err(Error::Down(e)) => Err(e),
    }
}

// ---------------------------------------------------------------------------
// JSON-RPC over the Unix socket

#[derive(Debug, PartialEq)]
enum Error {
    /// xapi unreachable, refused us, or answered nonsense.
    Down(String),
    /// An API error: code, then its parameters (e.g. ["UUID_INVALID", ...]).
    Api(Vec<String>),
}

impl From<std::io::Error> for Error {
    fn from(e: std::io::Error) -> Self {
        Error::Down(e.to_string())
    }
}

struct Rpc {
    path: PathBuf,
    session: Option<String>,
    id: u64,
}

impl Rpc {
    fn new(path: PathBuf) -> Self {
        Rpc {
            path,
            session: None,
            id: 0,
        }
    }

    /// Call `method` with the session prepended to `args`, logging in
    /// first if needed and once more if xapi forgot our session.
    fn call(&mut self, method: &str, args: &[Value]) -> Result<Value, Error> {
        let fresh = self.session.is_none();
        let s = self.login()?;
        let params = |s: String| {
            std::iter::once(Value::String(s))
                .chain(args.iter().cloned())
                .collect()
        };
        match self.raw(method, params(s)) {
            Err(Error::Api(e)) if !fresh && e.first().is_some_and(|c| c == "SESSION_INVALID") => {
                self.session = None;
                let s = self.login()?;
                self.raw(method, params(s))
            }
            r => r,
        }
    }

    fn login(&mut self) -> Result<String, Error> {
        if let Some(s) = &self.session {
            return Ok(s.clone());
        }
        let params = vec![
            "root".into(),
            "".into(),
            "1.0".into(),
            concat!("xentop-ng/", env!("CARGO_PKG_VERSION")).into(),
        ];
        let s = match self.raw("session.login_with_password", params) {
            Ok(Value::String(s)) => s,
            Ok(_) => return Err(Error::Down("login: unexpected answer".into())),
            Err(Error::Api(e)) => return Err(Error::Down(format!("login refused: {}", e.join(" ")))),
            Err(e) => return Err(e),
        };
        self.session = Some(s.clone());
        Ok(s)
    }

    fn logout(&mut self) {
        if let Some(s) = self.session.take() {
            let _ = self.raw("session.logout", vec![s.into()]);
        }
    }

    fn raw(&mut self, method: &str, params: Vec<Value>) -> Result<Value, Error> {
        self.id += 1;
        let body = json!({"jsonrpc": "2.0", "method": method, "params": params, "id": self.id});
        let resp = post(&self.path, body.to_string().as_bytes())?;
        parse_response(&resp)
    }
}

/// One HTTP POST to xapi's JSON-RPC endpoint, on a fresh connection.
fn post(path: &Path, body: &[u8]) -> Result<Vec<u8>, Error> {
    let mut s = UnixStream::connect(path).map_err(|e| Error::Down(format!("{}: {e}", path.display())))?;
    s.set_read_timeout(Some(IO_TIMEOUT))?;
    s.set_write_timeout(Some(IO_TIMEOUT))?;
    let head = format!(
        "POST /jsonrpc HTTP/1.1\r\nHost: localhost\r\nUser-Agent: xentop-ng/{}\r\n\
         Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
        env!("CARGO_PKG_VERSION"),
        body.len()
    );
    s.write_all(head.as_bytes())?;
    s.write_all(body)?;
    read_http(BufReader::new(s.take((MAX_RESPONSE + (64 << 10)) as u64)))
}

/// The body of an HTTP response, which must be a 200.
fn read_http(mut r: impl BufRead) -> Result<Vec<u8>, Error> {
    let bad = |what: &str| Error::Down(format!("bad HTTP response ({what})"));
    let mut line = String::new();
    r.read_line(&mut line)?;
    let code = line
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse::<u16>().ok())
        .ok_or_else(|| bad("status"))?;
    let (mut len, mut chunked) = (None, false);
    for _ in 0..=MAX_HEADERS {
        line.clear();
        if r.read_line(&mut line)? == 0 {
            return Err(bad("headers"));
        }
        let l = line.trim_end();
        if l.is_empty() {
            break;
        }
        let Some((k, v)) = l.split_once(':') else {
            continue;
        };
        match k.trim().to_ascii_lowercase().as_str() {
            "content-length" => len = Some(v.trim().parse::<usize>().map_err(|_| bad("length"))?),
            "transfer-encoding" => chunked = v.trim().eq_ignore_ascii_case("chunked"),
            _ => {}
        }
    }
    if code != 200 {
        return Err(Error::Down(format!("HTTP {code} from xapi")));
    }
    let mut body = Vec::new();
    if chunked {
        loop {
            line.clear();
            r.read_line(&mut line)?;
            let hex = line.trim().split(';').next().unwrap_or("");
            let n = usize::from_str_radix(hex, 16).map_err(|_| bad("chunk"))?;
            if n == 0 {
                break;
            }
            if body.len() + n > MAX_RESPONSE {
                return Err(bad("too long"));
            }
            let at = body.len();
            body.resize(at + n, 0);
            r.read_exact(&mut body[at..])?;
            line.clear();
            r.read_line(&mut line)?;
        }
    } else if let Some(n) = len {
        if n > MAX_RESPONSE {
            return Err(bad("too long"));
        }
        body.resize(n, 0);
        r.read_exact(&mut body)?;
    } else {
        r.take(MAX_RESPONSE as u64 + 1).read_to_end(&mut body)?;
        if body.len() > MAX_RESPONSE {
            return Err(bad("too long"));
        }
    }
    Ok(body)
}

/// The result of a JSON-RPC response, or its error. xapi puts its error
/// code in `message` and the parameters in `data`; JSON-RPC 1.0 style
/// replies carry the plain array instead.
fn parse_response(body: &[u8]) -> Result<Value, Error> {
    let mut v: Value =
        serde_json::from_slice(body).map_err(|e| Error::Down(format!("bad JSON-RPC reply: {e}")))?;
    let strings = |v: &Value| -> Vec<String> {
        v.as_array()
            .into_iter()
            .flatten()
            .map(|x| x.as_str().map(String::from).unwrap_or_else(|| x.to_string()))
            .collect()
    };
    match v.get("error") {
        None | Some(Value::Null) => {}
        Some(e @ Value::Array(_)) => return Err(Error::Api(strings(e))),
        Some(e) => {
            let code = e.get("message").and_then(Value::as_str).unwrap_or("UNKNOWN");
            let mut err = vec![code.to_string()];
            err.extend(e.get("data").map(strings).unwrap_or_default());
            return Err(Error::Api(err));
        }
    }
    match v.get_mut("result") {
        Some(r) => Ok(r.take()),
        None => Err(Error::Down("JSON-RPC reply without a result".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::{Backing, DomState, DomainRaw, NetRaw, VbdKind, VbdRaw};
    use std::os::unix::net::UnixListener;
    use std::time::Instant;

    const SR: &str = "ac70e429-0dec-3ccd-1d24-2713c6104b65";
    const VDI: &str = "86e4e84e-f30b-4a86-b634-1c38f02e1685";
    const VM: &str = "0eab0d7e-cdae-1386-91f5-2a06455f82f4";

    #[test]
    fn http_bodies() {
        let r = read_http(&b"HTTP/1.1 200 OK\r\nContent-Length: 4\r\n\r\nabcdEXTRA"[..]).unwrap();
        assert_eq!(r, b"abcd");
        let r = read_http(
            &b"HTTP/1.1 200 OK\r\nTransfer-Encoding: chunked\r\n\r\n3\r\nabc\r\n2;x=y\r\nde\r\n0\r\n\r\n"[..],
        );
        assert_eq!(r.unwrap(), b"abcde");
        let r = read_http(&b"HTTP/1.0 200 OK\r\n\r\nto the end"[..]).unwrap();
        assert_eq!(r, b"to the end");
        assert_eq!(
            read_http(&b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\n\r\n"[..]),
            Err(Error::Down("HTTP 404 from xapi".into()))
        );
        assert!(read_http(&b"garbage"[..]).is_err());
        assert!(read_http(&b"HTTP/1.1 200 OK\r\nContent-Length: 999999999999\r\n\r\n"[..]).is_err());
    }

    #[test]
    fn responses() {
        let ok = br#"{"jsonrpc":"2.0","result":"OpaqueRef:1","id":1}"#;
        assert_eq!(parse_response(ok), Ok(Value::String("OpaqueRef:1".into())));
        let err =
            br#"{"jsonrpc":"2.0","error":{"code":1,"message":"UUID_INVALID","data":["VDI","x"]},"id":1}"#;
        assert_eq!(
            parse_response(err),
            Err(Error::Api(vec!["UUID_INVALID".into(), "VDI".into(), "x".into()]))
        );
        let v1 = br#"{"result":null,"error":["SESSION_INVALID","OpaqueRef:1"],"id":1}"#;
        assert_eq!(
            parse_response(v1),
            Err(Error::Api(vec!["SESSION_INVALID".into(), "OpaqueRef:1".into()]))
        );
        assert!(matches!(parse_response(b"<html>"), Err(Error::Down(_))));
    }

    #[test]
    fn names_are_cleaned() {
        assert_eq!(clean("  Local storage \n", 64).as_deref(), Some("Local storage"));
        assert_eq!(clean("\x1b]0;pwn\x07", 64).as_deref(), Some("?]0;pwn?"));
        assert_eq!(clean("   ", 64), None);
        assert_eq!(clean(&"x".repeat(1000), 64).map(|s| s.len()), Some(64));
    }

    /// A pretend xapi: answers each JSON-RPC call through `answer`.
    fn fake_xapi(answer: fn(&str, &[Value]) -> Result<Value, Value>) -> (PathBuf, Arc<Mutex<Vec<String>>>) {
        static N: std::sync::atomic::AtomicU32 = std::sync::atomic::AtomicU32::new(0);
        let n = N.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!("xentop-ng-xapi-{}-{n}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let path = dir.join("xapi");
        let l = UnixListener::bind(&path).unwrap();
        let calls = Arc::new(Mutex::new(Vec::new()));
        let log = calls.clone();
        std::thread::spawn(move || {
            for s in l.incoming() {
                let Ok(s) = s else { break };
                let mut r = BufReader::new(s.try_clone().unwrap());
                let mut len = 0;
                let mut line = String::new();
                while r.read_line(&mut line).unwrap() > 2 {
                    if let Some(v) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        len = v.trim().parse().unwrap();
                    }
                    line.clear();
                }
                let mut body = vec![0; len];
                r.read_exact(&mut body).unwrap();
                let req: Value = serde_json::from_slice(&body).unwrap();
                let method = req["method"].as_str().unwrap().to_string();
                let params = req["params"].as_array().unwrap().clone();
                log.lock().unwrap().push(method.clone());
                let reply = match answer(&method, &params) {
                    Ok(v) => json!({"jsonrpc": "2.0", "result": v, "id": req["id"]}),
                    Err(e) => json!({"jsonrpc": "2.0", "error": {"code": 1, "message": e[0], "data": e.as_array().unwrap()[1..]}, "id": req["id"]}),
                }
                .to_string();
                let mut s = s;
                let _ = write!(
                    s,
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{reply}",
                    reply.len()
                );
            }
        });
        (path, calls)
    }

    fn snapshot() -> Snapshot {
        Snapshot {
            at: Instant::now(),
            hostname: "h".into(),
            xen_version: "4.17".into(),
            num_cpus: 1,
            cpu_hz: 0,
            tot_mem: 0,
            free_mem: 0,
            pcpu_idle_ns: None,
            domains: vec![DomainRaw {
                id: 1,
                name: "web01".into(),
                state: DomState::Running,
                flags: 0,
                ssid: 0,
                cpu_ns: 0,
                vcpus: vec![],
                cur_mem: 0,
                max_mem: 0,
                nets: vec![NetRaw {
                    id: 0,
                    ..Default::default()
                }],
                vbds: vec![VbdRaw {
                    dev: 51712,
                    kind: VbdKind::Vbd3,
                    oo_reqs: 0,
                    rd_reqs: 0,
                    wr_reqs: 0,
                    rd_sects: 0,
                    wr_sects: 0,
                    error: false,
                    ext: None,
                    backing: Some(Backing {
                        sr: Some(SR.into()),
                        vdi: Some(VDI.into()),
                        sr_kind: Some("lvm".into()),
                        ..Default::default()
                    }),
                }],
                vm_uuid: Some(VM.into()),
                mem_target: None,
                runnable_ns: None,
            }],
        }
    }

    /// Fill snapshots until `done` holds, or give up after a few seconds.
    fn fill_until(x: &mut Xapi, done: impl Fn(&Snapshot) -> bool) -> Snapshot {
        let t = Instant::now();
        loop {
            let mut s = snapshot();
            x.fill(&mut s);
            if done(&s) || t.elapsed() > Duration::from_secs(5) {
                return s;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
    }

    #[test]
    fn names_from_a_fake_xapi() {
        let (path, calls) = fake_xapi(|m, p| {
            let arg = p.get(1).and_then(Value::as_str).unwrap_or("");
            Ok(match m {
                "session.login_with_password" => {
                    assert_eq!(p[0], "root");
                    json!("OpaqueRef:session")
                }
                "SR.get_by_uuid" if arg == SR => json!("OpaqueRef:sr"),
                "SR.get_record" => json!({"name_label": "iSCSI\x1b[31m store", "type": "lvmoiscsi"}),
                "VDI.get_by_uuid" if arg == VDI => json!("OpaqueRef:vdi"),
                "VDI.get_name_label" => json!("web01 system disk"),
                "VM.get_by_uuid" if arg == VM => json!("OpaqueRef:vm"),
                "VM.get_VIFs" => json!(["OpaqueRef:vif0", "OpaqueRef:gone"]),
                "VIF.get_record" if arg == "OpaqueRef:vif0" => {
                    json!({"device": "0", "network": "OpaqueRef:net"})
                }
                "network.get_name_label" => json!("Pool-wide network associated with eth0"),
                "session.logout" => json!(""),
                _ => return Err(json!(["HANDLE_INVALID", m])),
            })
        });
        let mut x = Xapi::with_socket(path);
        let s = fill_until(&mut x, |s| {
            let d = &s.domains[0];
            d.vbds[0].backing.as_ref().unwrap().vdi_name.is_some() && d.nets[0].network.is_some()
        });
        let b = s.domains[0].vbds[0].backing.clone().unwrap();
        assert_eq!(b.sr_name.as_deref(), Some("iSCSI?[31m store"));
        assert_eq!(b.sr_kind.as_deref(), Some("lvmoiscsi"));
        assert_eq!(b.vdi_name.as_deref(), Some("web01 system disk"));
        assert_eq!(
            s.domains[0].nets[0].network.as_deref(),
            Some("Pool-wide network associated with eth0")
        );
        assert_eq!(x.state(), XapiState::Connected);
        // Nothing new wanted: no more calls on later samples.
        let n = calls.lock().unwrap().len();
        x.fill(&mut snapshot());
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(calls.lock().unwrap().len(), n);
        drop(x);
        let calls = calls.lock().unwrap();
        assert_eq!(
            calls
                .iter()
                .filter(|c| *c == "session.login_with_password")
                .count(),
            1
        );
        assert_eq!(calls.last().map(String::as_str), Some("session.logout"));
    }

    #[test]
    fn refused_login_is_reported() {
        let (path, _) = fake_xapi(|_, _| Err(json!(["SESSION_AUTHENTICATION_FAILED", "root"])));
        let mut x = Xapi::with_socket(path);
        let t = Instant::now();
        while matches!(x.state(), XapiState::Connecting) && t.elapsed() < Duration::from_secs(5) {
            x.fill(&mut snapshot());
            std::thread::sleep(Duration::from_millis(10));
        }
        let XapiState::Failed(why) = x.state() else {
            panic!("{:?}", x.state());
        };
        assert!(why.contains("SESSION_AUTHENTICATION_FAILED"), "{why}");
        // Unnamed, but otherwise untouched.
        let mut s = snapshot();
        x.fill(&mut s);
        assert_eq!(
            s.domains[0].vbds[0].backing.as_ref().unwrap().sr_kind.as_deref(),
            Some("lvm")
        );
    }

    #[test]
    fn no_xapi_socket() {
        assert!(!is_socket(Path::new("/nonexistent/xapi")));
        let mut x = Xapi::with_socket("/nonexistent/xapi".into());
        let t = Instant::now();
        while matches!(x.state(), XapiState::Connecting) && t.elapsed() < Duration::from_secs(5) {
            x.fill(&mut snapshot());
            std::thread::sleep(Duration::from_millis(10));
        }
        assert!(matches!(x.state(), XapiState::Failed(_)));
    }
}
