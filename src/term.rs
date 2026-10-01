//! Terminal setup and teardown that holds up on every exit path: normal
//! return, early `?` errors and panics.
//!
//! While the UI is up, stderr is pointed at /dev/null: libxenstat and
//! libxenctrl print diagnostics there (e.g. "Found interface vif3.0 but
//! domain 3 does not exist" while a VM shuts down), and since ratatui only
//! redraws cells that changed, such text would otherwise stay on screen.

use ratatui::crossterm::event::{DisableMouseCapture, EnableMouseCapture};
use ratatui::crossterm::execute;
use ratatui::DefaultTerminal;
use std::sync::atomic::{AtomicI32, Ordering};

/// Saved copy of the real stderr while it is redirected, or -1.
static SAVED_STDERR: AtomicI32 = AtomicI32::new(-1);

fn silence_stderr() {
    // SAFETY: plain fd juggling on fds we own; failures leave stderr as is.
    unsafe {
        let saved = libc::dup(2);
        if saved < 0 {
            return;
        }
        let null = libc::open(c"/dev/null".as_ptr(), libc::O_WRONLY | libc::O_CLOEXEC);
        if null < 0 {
            libc::close(saved);
            return;
        }
        libc::dup2(null, 2);
        libc::close(null);
        libc::fcntl(saved, libc::F_SETFD, libc::FD_CLOEXEC);
        SAVED_STDERR.store(saved, Ordering::SeqCst);
    }
}

fn restore_stderr() {
    let saved = SAVED_STDERR.swap(-1, Ordering::SeqCst);
    if saved >= 0 {
        // SAFETY: `saved` is the dup of the original stderr made above.
        unsafe {
            libc::dup2(saved, 2);
            libc::close(saved);
        }
    }
}

fn teardown() {
    let _ = execute!(std::io::stdout(), DisableMouseCapture);
    ratatui::restore();
    restore_stderr();
}

/// Owns the terminal's UI state; restores everything when dropped.
pub struct Guard;

impl Guard {
    pub fn enter() -> anyhow::Result<(DefaultTerminal, Guard)> {
        // ratatui::init() installs a panic hook that restores raw mode and
        // the alternate screen; wrap it so mouse capture and stderr come
        // back too, before the panic message is printed.
        let term = ratatui::init();
        let guard = Guard;
        let prev = std::panic::take_hook();
        std::panic::set_hook(Box::new(move |info| {
            let _ = execute!(std::io::stdout(), DisableMouseCapture);
            restore_stderr();
            prev(info);
        }));
        execute!(std::io::stdout(), EnableMouseCapture)?;
        silence_stderr();
        Ok((term, guard))
    }
}

impl Drop for Guard {
    fn drop(&mut self) {
        teardown();
    }
}
