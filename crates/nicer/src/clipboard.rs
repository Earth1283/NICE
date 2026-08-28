use std::sync::mpsc::{self, SyncSender};
use std::sync::Mutex;

use crate::error::{Error, Result};

/// The daemon owns the clipboard, but only through this trait, so that a headless node or
/// a frontend that owns the session can supply its own.
pub trait Clipboard: Send + Sync + 'static {
    fn read_text(&self) -> Result<String>;
    fn write_text(&self, text: String) -> Result<()>;
}

enum Request {
    Read(SyncSender<Result<String>>),
    Write(String, SyncSender<Result<()>>),
}

/// `arboard` must keep one long-lived handle: on X11 the process that set the selection is
/// the one that serves it, so a per-operation handle would publish content and immediately
/// withdraw it. The handle therefore lives on its own thread and is reached by channel.
pub struct SystemClipboard {
    requests: Mutex<SyncSender<Request>>,
}

impl SystemClipboard {
    pub fn spawn() -> Result<Self> {
        let (tx, rx) = mpsc::sync_channel::<Request>(8);
        let (ready_tx, ready_rx) = mpsc::sync_channel::<Result<()>>(1);

        std::thread::Builder::new()
            .name("nicer-clipboard".into())
            .spawn(move || {
                let mut clipboard = match arboard::Clipboard::new() {
                    Ok(clipboard) => {
                        let _ = ready_tx.send(Ok(()));
                        clipboard
                    }
                    Err(e) => {
                        let _ = ready_tx.send(Err(Error::Clipboard(e.to_string())));
                        return;
                    }
                };
                while let Ok(request) = rx.recv() {
                    match request {
                        Request::Read(reply) => {
                            let _ = reply.send(
                                clipboard
                                    .get_text()
                                    .map_err(|e| Error::Clipboard(e.to_string())),
                            );
                        }
                        Request::Write(text, reply) => {
                            let _ = reply.send(
                                clipboard
                                    .set_text(text)
                                    .map_err(|e| Error::Clipboard(e.to_string())),
                            );
                        }
                    }
                }
            })
            .map_err(|e| Error::Clipboard(format!("clipboard thread: {e}")))?;

        ready_rx
            .recv()
            .map_err(|_| Error::Clipboard("clipboard thread exited during startup".into()))??;

        Ok(Self {
            requests: Mutex::new(tx),
        })
    }

    fn dispatch<T>(&self, make: impl FnOnce(SyncSender<Result<T>>) -> Request) -> Result<T> {
        let (tx, rx) = mpsc::sync_channel(1);
        self.requests
            .lock()
            .map_err(|_| Error::Clipboard("clipboard channel poisoned".into()))?
            .send(make(tx))
            .map_err(|_| Error::Clipboard("clipboard thread is gone".into()))?;
        rx.recv()
            .map_err(|_| Error::Clipboard("clipboard thread dropped the request".into()))?
    }
}

impl Clipboard for SystemClipboard {
    fn read_text(&self) -> Result<String> {
        self.dispatch(Request::Read)
    }

    fn write_text(&self, text: String) -> Result<()> {
        self.dispatch(|reply| Request::Write(text, reply))
    }
}

/// Used when no display server is reachable, and in tests.
#[derive(Default)]
pub struct MemoryClipboard {
    text: Mutex<String>,
}

impl Clipboard for MemoryClipboard {
    fn read_text(&self) -> Result<String> {
        Ok(self.text.lock().expect("clipboard mutex").clone())
    }

    fn write_text(&self, text: String) -> Result<()> {
        *self.text.lock().expect("clipboard mutex") = text;
        Ok(())
    }
}

pub fn system_or_memory() -> Box<dyn Clipboard> {
    match SystemClipboard::spawn() {
        Ok(clipboard) => Box::new(clipboard),
        Err(e) => {
            tracing::warn!(error = %e, "no system clipboard; clipboard transfers stay in memory");
            Box::new(MemoryClipboard::default())
        }
    }
}
