//! A single owner for all collector handles. Construct them on the worker:
//! no Xen FFI pointer needs Send, and the UI never waits on a collector.
use super::{DataStatus, Source};
use crate::model::Snapshot;
use std::sync::mpsc::{self, Receiver, SyncSender, TryRecvError};

pub enum Update {
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
}

impl Collector {
    pub fn spawn(
        open: impl FnOnce() -> anyhow::Result<Box<dyn Source>> + Send + 'static,
        warmup_secs: u32,
    ) -> Self {
        let (requests, rx) = mpsc::sync_channel(1);
        let (tx, updates) = mpsc::sync_channel(1);
        let result = std::thread::Builder::new()
            .name("collector".into())
            .spawn(move || {
                let mut source = match open() {
                    Ok(s) => s,
                    Err(e) => {
                        let _ = tx.send(Update::Sample(Err(e.to_string())));
                        return;
                    }
                };
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
        // A failed spawn drops both worker endpoints; poll reports it.
        drop(result);
        Self {
            requests,
            updates,
            waiting: true,
            stopped: false,
        }
    }

    pub fn request(&mut self) {
        if !self.waiting && !self.stopped && self.requests.try_send(()).is_ok() {
            self.waiting = true;
        }
    }

    pub fn poll(&mut self) -> Option<Update> {
        if self.stopped {
            return None;
        }
        match self.updates.try_recv() {
            Ok(u) => {
                self.waiting = false;
                Some(u)
            }
            Err(TryRecvError::Empty) => None,
            Err(TryRecvError::Disconnected) => {
                self.stopped = true;
                Some(Update::Sample(Err("collector stopped".into())))
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
        assert!(matches!(await_update(&mut c), Update::Sample(Err(e)) if e == "cannot open Xen"));
    }
}
