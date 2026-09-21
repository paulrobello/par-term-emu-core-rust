//! The control-mode client: connect, auto-spawn, command, and pushed
//! notifications.
//!
//! Command replies and asynchronous notifications use separate paths: `send`
//! consumes `%begin`/`%end` blocks, while [`MuxClient::notifications`] carries
//! everything else, parsed. A reader that conflated the two would deadlock the
//! first time pane output arrived mid-command.

use crate::mux::ipc::{connect_local_stream, default_socket_path, LocalStream};
use crate::tmux_control::{TmuxControlParser, TmuxNotification};
use interprocess::TryClone as _;
use std::io::{self, BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::time::{Duration, Instant};

/// How long `send` waits for a reply block before giving up.
const REPLY_TIMEOUT: Duration = Duration::from_secs(10);
/// How long `connect_or_spawn_at` keeps retrying after spawning a daemon.
const SPAWN_CONNECT_DEADLINE: Duration = Duration::from_secs(10);

/// A control-mode client connected (or connectable) to a par-mux daemon.
pub struct MuxClient {
    writer: LocalStream,
    reply_rx: Receiver<Vec<String>>,
    notifications_rx: Receiver<TmuxNotification>,
}

impl MuxClient {
    /// Connect to a daemon already listening at `path`.
    pub fn connect(path: &Path) -> io::Result<Self> {
        let stream = connect_local_stream(path)?;
        Self::from_stream(stream)
    }

    /// The transparency entry point: connect to the default socket for `name`,
    /// starting a daemon if none is running.
    pub fn connect_or_spawn(name: &str) -> io::Result<Self> {
        Self::connect_or_spawn_at(&default_socket_path(name))
    }

    /// Connect to a daemon at `path`, spawning one when no live server owns it.
    ///
    /// Losing the spawn race is not an error: another client's daemon won the
    /// path, and connecting to the winner is the correct outcome.
    pub fn connect_or_spawn_at(path: &Path) -> io::Result<Self> {
        if let Ok(client) = Self::connect(path) {
            return Ok(client);
        }
        spawn_daemon(path);
        let deadline = Instant::now() + SPAWN_CONNECT_DEADLINE;
        loop {
            match Self::connect(path) {
                Ok(client) => return Ok(client),
                Err(_) if Instant::now() < deadline => {
                    std::thread::sleep(Duration::from_millis(50));
                }
                Err(last) => return Err(last),
            }
        }
    }

    fn from_stream(stream: LocalStream) -> io::Result<Self> {
        let mut writer = stream.try_clone()?;
        let (reply_tx, reply_rx) = channel::<Vec<String>>();
        let (notification_tx, notifications_rx) = channel::<TmuxNotification>();
        std::thread::spawn(move || reader_loop(stream, reply_tx, notification_tx));
        Ok(Self {
            writer,
            reply_rx,
            notifications_rx,
        })
    }

    /// Run one command and return its reply block's body lines.
    pub fn send(&mut self, command: &str) -> io::Result<Vec<String>> {
        writeln!(self.writer, "{command}")?;
        self.writer.flush()?;
        match self.reply_rx.recv_timeout(REPLY_TIMEOUT) {
            Ok(body) => Ok(body),
            Err(RecvTimeoutError::Timeout) => Err(io::Error::new(
                io::ErrorKind::TimedOut,
                format!("no reply block within {}s", REPLY_TIMEOUT.as_secs()),
            )),
            Err(RecvTimeoutError::Disconnected) => Err(io::Error::new(
                io::ErrorKind::ConnectionAborted,
                "server closed the connection",
            )),
        }
    }

    /// Pushed notifications (`%output` and friends), already parsed.
    pub fn notifications(&self) -> &Receiver<TmuxNotification> {
        &self.notifications_rx
    }
}

/// Split one connection's byte stream into reply blocks and notifications.
fn reader_loop(
    stream: LocalStream,
    reply_tx: Sender<Vec<String>>,
    notification_tx: Sender<TmuxNotification>,
) {
    let reader = BufReader::new(stream);
    let mut parser = TmuxControlParser::new(true);
    let mut block_body: Option<Vec<String>> = None;
    for line in reader.lines() {
        let Ok(line) = line else { break };
        if line.starts_with("%begin") {
            block_body = Some(Vec::new());
        } else if block_body.is_some() && (line.starts_with("%end") || line.starts_with("%error")) {
            let body = block_body.take().unwrap_or_default();
            if reply_tx.send(body).is_err() {
                break;
            }
        } else if let Some(body) = block_body.as_mut() {
            body.push(line);
        } else {
            // lines() strips the terminator; the parser only emits complete
            // lines, so hand it back the newline it buffers on.
            let mut framed = line.clone().into_bytes();
            framed.push(b'\n');
            let mut forwarded = true;
            for notification in parser.parse(&framed) {
                if notification_tx.send(notification).is_err() {
                    forwarded = false;
                }
            }
            if !forwarded {
                break;
            }
        }
    }
}

/// Start a par-mux daemon for `path`, next to our own executable.
///
/// Best effort: if the binary is missing the bounded retry in
/// [`MuxClient::connect_or_spawn_at`] turns that into a connect error rather
/// than a panic.
fn spawn_daemon(path: &Path) {
    let Ok(exe) = std::env::current_exe() else {
        return;
    };
    let Some(mut dir) = exe.parent().map(Path::to_path_buf) else {
        return;
    };
    // Integration-test binaries live in <target>/<profile>/deps while bins
    // sit in <target>/<profile> — walk out of deps to find the sibling bin.
    if dir.file_name() == Some(std::ffi::OsStr::new("deps")) {
        if let Some(parent) = dir.parent() {
            dir = parent.to_path_buf();
        }
    }
    #[cfg(unix)]
    let bin: PathBuf = dir.join("par-mux");
    #[cfg(windows)]
    let bin: PathBuf = dir.join("par-mux.exe");
    let _ = std::process::Command::new(bin)
        .arg("--socket")
        .arg(path)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}
