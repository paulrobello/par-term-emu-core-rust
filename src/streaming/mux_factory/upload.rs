//! Client uploads: [`MuxUploadPipe`], the per-session [`UploadForwarder`]
//! that drives the daemon's upload verbs over its own control connection.

use super::control::{command, MuxLines};
use super::mirror::MirrorLink;
use crate::mux::{connect_local_stream, LocalStream};
use crate::streaming::server::UploadForwarder;
use base64::Engine as _;
use interprocess::TryClone as _;
use parking_lot::Mutex;
use std::io;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

/// One session's client-upload pipe ([`UploadForwarder`]). The upload
/// verbs run on their OWN short-lived daemon control connection — the
/// mirror link's connection is read solely by its drain thread, so a
/// request/reply verb cannot share it (the same shape
/// [`follow_active_workspace`](super::mirror::follow_active_workspace) resolves over).
///
/// Ordering is the point: a client may send `upload_chunk` and
/// `upload_commit` back-to-back, so the verbs are serialized through ONE
/// bounded op queue drained by ONE worker thread (spawned with the first
/// verb). The worker alone owns the daemon connection, every verb's reply
/// block is read there, and the pane is resolved at COMMIT time from the
/// link so a retargeted workspace-active mirror pastes into the pane it
/// currently mirrors. Callers block (bounded by the reply deadline) on a
/// one-shot reply channel — the same bounded-wait shape the input path's
/// blocking writer accepts.
pub struct MuxUploadPipe {
    link: Arc<MirrorLink>,
    socket: PathBuf,
    reply_timeout: Duration,
    /// The op queue + its worker, created lazily by the first verb.
    ops: Mutex<Option<OpsHandle>>,
    /// The daemon-side id of the current/last upload: written by the
    /// worker, read by the handlers for the `upload_finished` reports.
    upload_id: Arc<Mutex<Option<u64>>>,
}

struct OpsHandle {
    sender: std::sync::mpsc::SyncSender<UploadOp>,
    _worker: std::thread::JoinHandle<()>,
}

/// One queued verb. `reply` is the caller's one-shot channel; the worker
/// sends exactly once per op.
enum UploadOp {
    Begin {
        filename: String,
        total_bytes: u64,
        reply: std::sync::mpsc::SyncSender<io::Result<u64>>,
    },
    Chunk {
        data: Vec<u8>,
        reply: std::sync::mpsc::SyncSender<io::Result<()>>,
    },
    Commit {
        reply: std::sync::mpsc::SyncSender<io::Result<String>>,
    },
    Abort {
        reply: std::sync::mpsc::SyncSender<io::Result<()>>,
    },
}

/// Verbs queued ahead of the worker before a producer blocks — dozens of
/// full-size chunks of slack; the producer (the session's ws task) is the
/// backpressure point past that.
const UPLOAD_OP_QUEUE_DEPTH: usize = 64;

/// The worker's exclusive daemon connection pair for one upload's
/// lifetime.
struct PipeConn {
    writer: LocalStream,
    reader: MuxLines,
}

/// The worker's exclusive daemon state: one connection pair for the
/// upload's lifetime and the shared id slot.
struct UploadWorker {
    conn: Option<PipeConn>,
    upload_id: Arc<Mutex<Option<u64>>>,
    link: Arc<MirrorLink>,
    socket: PathBuf,
    reply_timeout: Duration,
}

impl UploadWorker {
    fn connect(&mut self) -> io::Result<PipeConn> {
        let stream = connect_local_stream(&self.socket).inspect_err(|_| {
            crate::streaming::metrics::Metrics::global().mux_socket_error("upload-connect");
        })?;
        let writer = stream.try_clone()?;
        let reader = MuxLines::spawn(stream)?;
        Ok(PipeConn { writer, reader })
    }

    fn run(mut self, receiver: std::sync::mpsc::Receiver<UploadOp>) {
        while let Ok(op) = receiver.recv() {
            match op {
                UploadOp::Begin {
                    filename,
                    total_bytes,
                    reply,
                } => {
                    let _ = reply.send(self.begin(&filename, total_bytes));
                }
                UploadOp::Chunk { data, reply } => {
                    let _ = reply.send(self.chunk(&data));
                }
                UploadOp::Commit { reply } => {
                    let _ = reply.send(self.commit());
                }
                UploadOp::Abort { reply } => {
                    let _ = reply.send(self.abort());
                }
            }
        }
    }

    fn begin(&mut self, filename: &str, total_bytes: u64) -> io::Result<u64> {
        if self.conn.is_some() {
            return Err(io::Error::other(
                "an upload is already in progress on this session",
            ));
        }
        let mut fresh = self.connect()?;
        let reply = command(
            &mut fresh.writer,
            &fresh.reader,
            &format!("upload-begin {} {total_bytes}", quote_upload_word(filename)),
            self.reply_timeout,
        )?;
        let upload_id = reply
            .first()
            .and_then(|line| line.trim().parse::<u64>().ok())
            .ok_or_else(|| {
                io::Error::other(format!("upload-begin reply was not an id: {reply:?}"))
            })?;
        *self.upload_id.lock() = Some(upload_id);
        self.conn = Some(fresh);
        Ok(upload_id)
    }

    fn chunk(&mut self, data: &[u8]) -> io::Result<()> {
        let Some(conn) = self.conn.as_mut() else {
            return Err(io::Error::other("no upload in progress on this session"));
        };
        let Some(upload_id) = *self.upload_id.lock() else {
            return Err(io::Error::other("no upload id"));
        };
        let encoded = base64::engine::general_purpose::STANDARD.encode(data);
        command(
            &mut conn.writer,
            &conn.reader,
            &format!("upload-chunk {upload_id} {encoded}"),
            self.reply_timeout,
        )?;
        Ok(())
    }

    fn commit(&mut self) -> io::Result<String> {
        let Some(mut conn) = self.conn.take() else {
            return Err(io::Error::other("no upload in progress on this session"));
        };
        let Some(upload_id) = *self.upload_id.lock() else {
            return Err(io::Error::other("no upload id"));
        };
        let pane = self.link.pane();
        let reply = command(
            &mut conn.writer,
            &conn.reader,
            // The commit grammar reads LEADING flags only: -t first, then
            // the upload id (MUX.md's Command Reference row).
            &format!("upload-commit -t %{pane} {upload_id}"),
            self.reply_timeout,
        )?;
        let path = reply.first().cloned().unwrap_or_default();
        if path.is_empty() {
            return Err(io::Error::other("upload-commit reply carried no path"));
        }
        Ok(path)
    }

    fn abort(&mut self) -> io::Result<()> {
        let Some(mut conn) = self.conn.take() else {
            return Err(io::Error::other("no upload in progress on this session"));
        };
        if let Some(upload_id) = *self.upload_id.lock() {
            let _ = command(
                &mut conn.writer,
                &conn.reader,
                &format!("upload-abort {upload_id}"),
                self.reply_timeout,
            );
        }
        Ok(())
    }
}

impl MuxUploadPipe {
    pub(super) fn new(link: Arc<MirrorLink>, socket: PathBuf, reply_timeout: Duration) -> Self {
        Self {
            link,
            socket,
            reply_timeout,
            ops: Mutex::new(None),
            upload_id: Arc::new(Mutex::new(None)),
        }
    }

    /// Queue one op and wait (bounded) for the worker's reply. The op
    /// queue and its worker are created by the first call; the worker is
    /// a plain thread (the verbs block on daemon I/O) that exits when the
    /// pipe — and with it the sender — drops.
    fn call<T>(
        &self,
        make: impl FnOnce(std::sync::mpsc::SyncSender<io::Result<T>>) -> UploadOp,
    ) -> io::Result<T> {
        let (reply_tx, reply_rx) = std::sync::mpsc::sync_channel(1);
        {
            let mut ops = self.ops.lock();
            if ops.is_none() {
                let (sender, receiver) =
                    std::sync::mpsc::sync_channel::<UploadOp>(UPLOAD_OP_QUEUE_DEPTH);
                let worker = UploadWorker {
                    conn: None,
                    upload_id: Arc::clone(&self.upload_id),
                    link: Arc::clone(&self.link),
                    socket: self.socket.clone(),
                    reply_timeout: self.reply_timeout,
                };
                let handle = std::thread::Builder::new()
                    .name("mux-upload-worker".to_string())
                    .spawn(move || worker.run(receiver))
                    .map_err(|err| {
                        io::Error::other(format!("cannot spawn upload worker: {err}"))
                    })?;
                *ops = Some(OpsHandle {
                    sender,
                    _worker: handle,
                });
            }
            ops.as_ref()
                .expect("just initialized")
                .sender
                .send(make(reply_tx))
                .map_err(|_| io::Error::other("the upload worker is gone"))?;
        }
        reply_rx.recv_timeout(self.reply_timeout).map_err(|_| {
            io::Error::new(
                io::ErrorKind::TimedOut,
                "no upload verb reply within the deadline",
            )
        })?
    }
}

/// One shell-quoted word for a control line (the bounded quoting grammar
/// the daemon parses: single quotes with the close-escape-reopen idiom),
/// so a filename with spaces survives the wire.
fn quote_upload_word(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

impl UploadForwarder for MuxUploadPipe {
    fn begin(&self, filename: &str, total_bytes: u64) -> io::Result<u64> {
        self.call(|reply| UploadOp::Begin {
            filename: filename.to_string(),
            total_bytes,
            reply,
        })
    }

    fn chunk(&self, data: &[u8]) -> io::Result<()> {
        self.call(|reply| UploadOp::Chunk {
            data: data.to_vec(),
            reply,
        })
    }

    fn commit(&self) -> io::Result<String> {
        self.call(|reply| UploadOp::Commit { reply })
    }

    fn abort(&self) -> io::Result<()> {
        self.call(|reply| UploadOp::Abort { reply })
    }

    fn upload_id(&self) -> Option<u64> {
        *self.upload_id.lock()
    }
}
