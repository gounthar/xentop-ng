//! A single owner for all collector handles. Construct them on the worker:
//! no Xen FFI pointer needs Send, and the UI never waits on a collector.
use super::{DataStatus, Source};
use crate::model::Snapshot;
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TryRecvError};
use std::time::Duration;

pub enum Update {
    /// Initialization failure or unexpected worker termination.
    Fatal(String),
    Ready {
        history: Vec<Snapshot>,
        status: DataStatus,
        description: String,
    },
    Sample(Result<(Snapshot, DataStatus), String>),
}

pub struct Collector {
    requests: SyncSender<()>,
    updates: Receiver<Update>,
    waiting: bool,
    stopped: bool,
    startup: Receiver<Result<(), String>>,
    starting: bool,
    last_error: Option<String>,
    spawn_error: Option<String>,
}

impl Collector {
    pub fn spawn(
        open: impl FnOnce() -> anyhow::Result<Box<dyn Source>> + Send + 'static,
        warmup_secs: u32,
    ) -> Self {
        let (startup_tx, startup) = mpsc::sync_channel(1);
        let (requests, rx) = mpsc::sync_channel(1);
        let (tx, updates) = mpsc::sync_channel(1);
        let result = std::thread::Builder::new()
            .name("collector".into())
            .spawn(move || {
                let mut source = match open() {
                    Ok(s) => s,
                    Err(e) => {
                        let _ = startup_tx.send(Err(format!("{e:#}")));
                        return;
                    }
                };
                if startup_tx.send(Ok(())).is_err() {
                    return;
                }
                let ready = Update::Ready {
                    history: source.warmup(warmup_secs),
                    status: source.status(),
                    description: source.describe(),
                };
                if tx.send(ready).is_err() {
                    return;
                }
                while rx.recv().is_ok() {
                    let sample = source
                        .sample()
                        .map(|s| (s, source.status()))
                        .map_err(|e| e.to_string());
                    if tx.send(Update::Sample(sample)).is_err() {
                        break;
                    }
                }
            });
        let spawn_error = result.err().map(|e| format!("cannot start collector: {e}"));
        Self {
            requests,
            updates,
            waiting: true,
            stopped: false,
            startup,
            starting: true,
            last_error: None,
            spawn_error,
        }
    }

    /// Wait only briefly before opening the terminal. A slow initializer
    /// continues on the worker; poll reports its eventual result in the UI.
    pub fn wait_started(&mut self, timeout: Duration) -> Result<bool, String> {
        if let Some(e) = self.spawn_error.take() {
            self.stopped = true;
            return Err(e);
        }
        if !self.starting {
            return Ok(true);
        }
        match self.startup.recv_timeout(timeout) {
            Ok(Ok(())) => {
                self.starting = false;
                Ok(true)
            }
            Ok(Err(e)) => {
                self.stopped = true;
                Err(e)
            }
            Err(RecvTimeoutError::Timeout) => Ok(false),
            Err(RecvTimeoutError::Disconnected) => {
                self.stopped = true;
                Err("collector stopped during initialization".into())
            }
        }
    }

    pub fn request(&mut self) -> bool {
        if !self.waiting && !self.stopped && self.requests.try_send(()).is_ok() {
            self.waiting = true;
            true
        } else {
            false
        }
    }

    pub fn waiting(&self) -> bool {
        self.waiting && !self.stopped
    }

    pub fn poll(&mut self) -> Option<Update> {
        if self.stopped {
            return None;
        }
        match self.wait_started(Duration::ZERO) {
            Ok(false) => return None,
            Err(e) => return Some(Update::Fatal(e)),
            Ok(true) => {}
        }
        match self.updates.try_recv() {
            Ok(u) => {
                self.waiting = false;
                if let Update::Sample(result) = &u {
                    self.last_error = result.as_ref().err().cloned();
                }
                Some(u)
            }
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                // The worker only exits on its own by panicking (release
                // builds abort instead). A sampling error is context, not
                // the cause.
                self.stopped = true;
                Some(Update::Fatal(match self.last_error.take() {
                    Some(e) => format!("collector stopped unexpectedly (last sample error: {e})"),
                    None => "collector stopped unexpectedly".into(),
                }))
            }
        }
    }
}

// Deliberately do not join on Drop: a blocked foreign call cannot be
// cancelled safely. Disconnection ends an idle worker; process exit ends
// a stuck one. A restartable process boundary is separate future work.
#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{Duration, Instant};

    struct Blocked {
        release: Receiver<()>,
        started: mpsc::Sender<()>,
    }
    impl Source for Blocked {
        fn sample(&mut self) -> anyhow::Result<Snapshot> {
            self.started.send(()).unwrap();
            self.release.recv().unwrap();
            anyhow::bail!("released")
        }
        fn describe(&self) -> String {
            "blocked fixture".into()
        }
    }
    fn await_update(c: &mut Collector) -> Update {
        let until = Instant::now() + Duration::from_secs(2);
        loop {
            if let Some(u) = c.poll() {
                return u;
            }
            assert!(Instant::now() < until);
            std::thread::sleep(Duration::from_millis(1));
        }
    }
    #[test]
    fn blocked_sample_does_not_block_poll_or_drop_or_queue_work() {
        let (release, rx) = mpsc::channel();
        let (started, start_rx) = mpsc::channel();
        let mut c = Collector::spawn(move || Ok(Box::new(Blocked { release: rx, started })), 0);
        assert!(matches!(await_update(&mut c), Update::Ready { .. }));
        c.request();
        start_rx.recv_timeout(Duration::from_secs(2)).unwrap();
        let now = Instant::now();
        for _ in 0..100 {
            assert!(c.poll().is_none());
            c.request();
        }
        drop(c);
        assert!(now.elapsed() < Duration::from_millis(200));
        release.send(()).unwrap();
        assert!(start_rx.recv_timeout(Duration::from_secs(2)).is_err());
    }
    #[test]
    fn initialization_error_is_reported() {
        let mut c = Collector::spawn(|| anyhow::bail!("cannot open Xen"), 0);
        assert!(matches!(await_update(&mut c), Update::Fatal(e) if e == "cannot open Xen"));
        for _ in 0..10 {
            assert!(c.poll().is_none());
        }
    }

    #[test]
    fn startup_wait_returns_the_original_error() {
        let mut c = Collector::spawn(|| anyhow::bail!("missing library"), 0);
        assert_eq!(
            c.wait_started(Duration::from_secs(2)),
            Err("missing library".into())
        );
        assert!(c.poll().is_none());
    }

    #[test]
    fn slow_initialization_can_be_abandoned() {
        let (tx, rx) = mpsc::channel();
        let mut c = Collector::spawn(
            move || {
                rx.recv().unwrap();
                anyhow::bail!("late failure")
            },
            0,
        );
        assert_eq!(c.wait_started(Duration::from_millis(1)), Ok(false));
        tx.send(()).unwrap();
        assert!(matches!(await_update(&mut c), Update::Fatal(e) if e == "late failure"));
        assert!(c.poll().is_none());
    }

    #[test]
    fn disconnect_reports_termination_with_last_error_as_context() {
        let (requests, _rx) = mpsc::sync_channel(1);
        let (tx, updates) = mpsc::sync_channel(1);
        let (_startup_tx, startup) = mpsc::sync_channel(1);
        let mut c = Collector {
            requests,
            updates,
            waiting: true,
            stopped: false,
            startup,
            starting: false,
            last_error: None,
            spawn_error: None,
        };
        tx.send(Update::Sample(Err("specific read error".into())))
            .unwrap();
        drop(tx);
        assert!(matches!(c.poll(), Some(Update::Sample(Err(_)))));
        assert!(
            matches!(c.poll(), Some(Update::Fatal(e)) if e == "collector stopped unexpectedly (last sample error: specific read error)")
        );
        assert!(c.poll().is_none());
    }
}
