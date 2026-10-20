//! Raw pane streams for local clients: the client half of `observe-pane`
//! (read-only, any number of observers per pane) and `control-pane`
//! (writable: stdin bytes become `send-keys -H`, size reports become
//! `refresh-client -t %N -C WxH`).
//!
//! The wire is the control socket's own grammar — the stream reply's body
//! is `pane %N COLSxROWS[ exited=CODE]` then the pane's screen-restore
//! seed, and after it the connection carries only that pane's `%output`,
//! `%pane-exited`, `%pane-respawned`, `%exit`, and the connection's own
//! reply blocks. `%output` that arrives BEFORE the stream reply's `%end`
//! is already in the seed (the daemon queues the seed and the narrowing
//! atomically against pushes), so a consumer drops it and applies only
//! what follows. [`MuxClient`](crate::mux::MuxClient) splits replies and
//! notifications onto separate channels and loses that order, so this
//! module reads the socket itself.

use crate::mux::ipc::{connect_local_stream, LocalStream};
use crate::mux::server::{fill_line_bounded, is_poll_wake, LineFill};
use interprocess::TryClone as _;
use par_term_emu_core::tmux_control::{TmuxControlParser, TmuxNotification};
use std::io::{self, BufReader, Read, Write};
use std::path::Path;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

/// Per-line byte budget for the stream reader: a seed reply carries the
/// pane's whole scrollback, so this is larger than the command client's.
/// cap: Bytes accumulated from one daemon line by a pane stream client before it closes the connection.
const MAX_STREAM_LINE_BYTES: usize = 64 * 1024 * 1024;

/// The largest stdin chunk one `send-keys -H` carries: the hex form
/// triples it, keeping each line far under the daemon's 1 MiB budget.
/// cap: Stdin bytes a control-pane client forwards per send-keys line.
const CONTROL_CHUNK: usize = 4096;

/// How often `control` re-reads the controlling terminal's size.
const SIZE_POLL: Duration = Duration::from_millis(250);

/// One event of a pane stream, in wire order.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamEvent {
    /// The stream reply: the pane, its grid at seed time, its exit code if
    /// it was already held dead, and the screen-restore seed bytes.
    Seed {
        /// `%N`.
        pane: String,
        /// The pane's grid at seed time.
        size: (u16, u16),
        /// `Some` when the pane was held dead at seed time (`None` inside
        /// = the code was unreadable).
        exited: Option<Option<i32>>,
        /// Bytes that reproduce the pane's screen and scrollback.
        seed: Vec<u8>,
    },
    /// Raw PTY output that followed the seed.
    Output(Vec<u8>),
    /// The pane's process exited (it may be held for a respawn).
    Exited(Option<i32>),
    /// The pane's process was respawned in place: a fresh terminal.
    Respawned,
    /// One of the connection's own later commands was answered.
    Reply {
        /// `%end` (true) or `%error` (false).
        ok: bool,
        /// The reply body.
        body: Vec<String>,
    },
    /// The stream ended: `%exit` (the pane closed or the daemon is ending).
    Closed,
}

/// The reader side of a pane stream: decodes the socket into
/// [`StreamEvent`]s in wire order.
pub struct StreamReader {
    reader: BufReader<LocalStream>,
    decoder: StreamDecoder,
}

/// The line-to-event half of [`StreamReader`], transport-free.
#[derive(Default)]
pub struct StreamDecoder {
    parser: Option<TmuxControlParser>,
    seeded: bool,
    open: Option<(u32, Vec<String>)>,
}

impl StreamReader {
    /// The next event, or `Ok(None)` at EOF.
    pub fn next_event(&mut self) -> io::Result<Option<StreamEvent>> {
        loop {
            let mut buf = Vec::new();
            loop {
                match fill_line_bounded(&mut self.reader, &mut buf, MAX_STREAM_LINE_BYTES) {
                    Ok(LineFill::Complete) => break,
                    Ok(LineFill::Eof) if buf.is_empty() => return Ok(None),
                    Ok(LineFill::Eof) => break,
                    Ok(LineFill::Oversize) => {
                        return Err(io::Error::new(
                            io::ErrorKind::InvalidData,
                            "daemon line exceeded the stream budget",
                        ))
                    }
                    Err(err) if is_poll_wake(&err) => continue,
                    Err(err) => return Err(err),
                }
            }
            if buf.last() == Some(&b'\n') {
                buf.pop();
            }
            if let Some(event) = self.decoder.on_line(&String::from_utf8_lossy(&buf)) {
                return Ok(Some(event));
            }
        }
    }
}

impl StreamDecoder {
    /// Decode one line (its `\n` stripped): `Some` when it completes an
    /// event. Seed body lines keep any `\r` — the restore stream's bytes
    /// join back exactly.
    pub fn on_line(&mut self, line: &str) -> Option<StreamEvent> {
        match framing(line) {
            Some(("%begin", number)) if self.open.is_none() => {
                self.open = Some((number, Vec::new()));
                return None;
            }
            Some((kind @ ("%end" | "%error"), number))
                if self.open.as_ref().is_some_and(|(n, _)| *n == number) =>
            {
                let (_, body) = self.open.take().expect("guarded open block");
                let ok = kind == "%end";
                if self.seeded {
                    return Some(StreamEvent::Reply { ok, body });
                }
                if !ok {
                    return Some(StreamEvent::Reply { ok, body });
                }
                self.seeded = true;
                return Some(seed_event(&body));
            }
            _ => {}
        }
        if let Some((_, body)) = self.open.as_mut() {
            body.push(line.to_string());
            return None;
        }
        let mut framed = line.trim_end_matches('\r').as_bytes().to_vec();
        framed.push(b'\n');
        let notification = self
            .parser
            .get_or_insert_with(|| TmuxControlParser::new(true))
            .parse(&framed)
            .into_iter()
            .next()?;
        match notification {
            // Already in the seed: the daemon queued it ahead of the reply.
            TmuxNotification::Output { .. } if !self.seeded => None,
            TmuxNotification::Output { data, .. } => Some(StreamEvent::Output(data)),
            TmuxNotification::PaneExited { exit_code, .. } if self.seeded => {
                Some(StreamEvent::Exited(exit_code))
            }
            TmuxNotification::PaneRespawned { .. } if self.seeded => Some(StreamEvent::Respawned),
            TmuxNotification::Exit => Some(StreamEvent::Closed),
            _ => None,
        }
    }
}

/// `%begin`/`%end`/`%error` with its command number.
fn framing(line: &str) -> Option<(&str, u32)> {
    let mut fields = line.split_whitespace();
    let kind = fields.next()?;
    if !matches!(kind, "%begin" | "%end" | "%error") {
        return None;
    }
    let _timestamp = fields.next()?;
    Some((kind, fields.next()?.parse().ok()?))
}

/// Decode the stream reply's body: the `pane %N COLSxROWS[ exited=CODE]`
/// header line, then the seed. The seed's own lines join back exactly
/// (the restore stream's line feeds are the newlines the wire split on).
fn seed_event(body: &[String]) -> StreamEvent {
    let header = body.first().map(String::as_str).unwrap_or_default();
    let mut fields = header.split_whitespace().skip(1);
    let pane = fields.next().unwrap_or_default().to_string();
    let size = fields
        .next()
        .and_then(|dims| dims.split_once('x'))
        .and_then(|(c, r)| Some((c.parse().ok()?, r.parse().ok()?)))
        .unwrap_or((0, 0));
    let exited = fields
        .find_map(|field| field.strip_prefix("exited="))
        .map(|code| code.parse().ok());
    let seed = body.get(1..).unwrap_or_default().join("\n").into_bytes();
    StreamEvent::Seed {
        pane,
        size,
        exited,
        seed,
    }
}

/// Open a pane stream on the daemon at `socket`: `control = false` sends
/// `observe-pane -t <target>`, `true` sends `control-pane -t <target>`.
/// Returns the reader and, for a control stream, the writer the input and
/// size reports go through.
pub fn open(
    socket: &Path,
    target: &str,
    control: bool,
) -> io::Result<(StreamReader, Option<StreamWriter>)> {
    let stream = connect_local_stream(socket)?;
    let mut writer = stream.try_clone()?;
    let verb = if control {
        "control-pane"
    } else {
        "observe-pane"
    };
    writeln!(writer, "{verb} -t {target}")?;
    writer.flush()?;
    let reader = StreamReader {
        reader: BufReader::new(stream),
        decoder: StreamDecoder::default(),
    };
    let writer = control.then(|| StreamWriter {
        writer,
        pane: String::new(),
    });
    Ok((reader, writer))
}

/// The writer side of a `control-pane` stream.
pub struct StreamWriter {
    writer: LocalStream,
    pane: String,
}

impl StreamWriter {
    /// Address later writes at `pane` (`%N`, from the seed).
    pub fn set_pane(&mut self, pane: &str) {
        self.pane = pane.to_string();
    }

    /// Forward raw input bytes to the pane, chunked into `send-keys -H`
    /// lines.
    pub fn input(&mut self, bytes: &[u8]) -> io::Result<()> {
        for chunk in bytes.chunks(CONTROL_CHUNK) {
            let mut line = format!("send-keys -t {} -H", self.pane);
            for byte in chunk {
                line.push_str(&format!(" {byte:02x}"));
            }
            writeln!(self.writer, "{line}")?;
        }
        self.writer.flush()
    }

    /// Report the controller's grid: the pane's window takes the minimum
    /// over every client viewing it, and this contribution is withdrawn
    /// when the connection closes.
    pub fn resize(&mut self, cols: u16, rows: u16) -> io::Result<()> {
        writeln!(
            self.writer,
            "refresh-client -t {} -C {cols}x{rows}",
            self.pane
        )?;
        self.writer.flush()
    }
}

/// What `par-mux observe`/`par-mux control` do with the stream.
#[derive(Debug, Clone, Default)]
pub struct StreamOptions {
    /// `control-pane` instead of `observe-pane`.
    pub control: bool,
    /// A fixed size report (`--size WxH`) instead of following the
    /// controlling terminal.
    pub size: Option<(u16, u16)>,
    /// Write no seed: stdout carries only the output that follows.
    pub no_seed: bool,
}

/// What the stream runner's main loop waits on.
enum Wake {
    Event(io::Result<Option<StreamEvent>>),
    /// A controller's stdin reached EOF.
    InputDone,
}

/// Run a pane stream to stdout until it ends, returning the process exit
/// code: 0 when the daemon ended it with `%exit` (the pane closed, or a
/// shutdown) or a controller's stdin reached EOF; 1 when it was refused
/// (no daemon, no such pane, a hook-only socket); 2 when the connection
/// dropped mid-stream without `%exit` (eviction, a killed daemon) or
/// failed. The caller exits with
/// it; returning drops the connection, which withdraws a controller's
/// size contribution daemon-side.
pub fn run(socket: &Path, target: &str, options: &StreamOptions) -> i32 {
    let (mut reader, mut writer) = match open(socket, target, options.control) {
        Ok(opened) => opened,
        Err(err) => {
            eprintln!("par-mux: no daemon running on {} ({err})", socket.display());
            return 1;
        }
    };
    let (wake_tx, wake) = std::sync::mpsc::channel::<Wake>();
    let events = wake_tx.clone();
    std::thread::spawn(move || loop {
        let event = reader.next_event();
        let last = !matches!(event, Ok(Some(_)));
        if events.send(Wake::Event(event)).is_err() || last {
            return;
        }
    });
    let mut seeded = false;
    let mut stdout = io::stdout().lock();
    loop {
        let event = match wake.recv() {
            Ok(Wake::Event(Ok(Some(event)))) => event,
            Ok(Wake::InputDone) => return 0,
            // The daemon ends every stream it ends on purpose with %exit
            // (handled below), so a bare EOF is a refusal (a hook-only
            // pane endpoint answers a command with one JSON line and
            // closes) or a lost connection (eviction, a killed daemon).
            Ok(Wake::Event(Ok(None))) | Err(_) if !seeded => {
                eprintln!(
                    "par-mux: the daemon closed the connection without a stream reply — a \
                     hook-only pane socket refuses streams; pass --socket for the daemon's \
                     control socket"
                );
                return 1;
            }
            Ok(Wake::Event(Ok(None))) | Err(_) => {
                eprintln!("par-mux: the stream's connection closed without %exit");
                return 2;
            }
            Ok(Wake::Event(Err(err))) => {
                eprintln!("par-mux: stream failed: {err}");
                return 2;
            }
        };
        let wrote = match event {
            StreamEvent::Seed { pane, seed, .. } => {
                seeded = true;
                if let Some(writer) = writer.take() {
                    start_control(writer, &pane, options.size, wake_tx.clone());
                }
                if options.no_seed {
                    Ok(())
                } else {
                    stdout.write_all(&seed)
                }
            }
            StreamEvent::Output(bytes) => stdout.write_all(&bytes),
            StreamEvent::Reply { ok: false, body } => {
                eprintln!("par-mux: {}", body.join(" "));
                if !seeded {
                    return 1;
                }
                Ok(())
            }
            StreamEvent::Closed => return 0,
            StreamEvent::Exited(_) | StreamEvent::Respawned | StreamEvent::Reply { .. } => Ok(()),
        };
        // A closed stdout (`| head`) ends the stream quietly.
        if wrote.and_then(|()| stdout.flush()).is_err() {
            return 0;
        }
    }
}

/// The control stream's input side: stdin bytes forwarded until EOF, and
/// size reports — the fixed `--size`, or the controlling terminal's size
/// re-read every [`SIZE_POLL`] and reported when it changes. Stdin EOF
/// wakes the runner, which returns and so closes the connection.
fn start_control(
    mut writer: StreamWriter,
    pane: &str,
    fixed: Option<(u16, u16)>,
    wake: std::sync::mpsc::Sender<Wake>,
) {
    writer.set_pane(pane);
    let initial = fixed.or_else(terminal_size);
    if let Some((cols, rows)) = initial {
        let _ = writer.resize(cols, rows);
    }
    let done = Arc::new(AtomicBool::new(false));
    if fixed.is_none() {
        if let Ok(stream) = writer.writer.try_clone() {
            let mut resizer = StreamWriter {
                writer: stream,
                pane: pane.to_string(),
            };
            let done = Arc::clone(&done);
            std::thread::spawn(move || {
                let mut last = initial;
                while !done.load(Ordering::Relaxed) {
                    std::thread::sleep(SIZE_POLL);
                    let now = terminal_size();
                    if let Some((cols, rows)) = now.filter(|_| now != last) {
                        if resizer.resize(cols, rows).is_err() {
                            return;
                        }
                        last = now;
                    }
                }
            });
        }
    }
    std::thread::spawn(move || {
        let mut stdin = io::stdin().lock();
        let mut buf = [0u8; CONTROL_CHUNK];
        loop {
            match stdin.read(&mut buf) {
                Ok(0) | Err(_) => break,
                Ok(n) => {
                    if writer.input(&buf[..n]).is_err() {
                        break;
                    }
                }
            }
        }
        done.store(true, Ordering::Relaxed);
        let _ = wake.send(Wake::InputDone);
    });
}

/// The controlling terminal's grid, read from stdout (then stdin) —
/// `None` when neither is a terminal.
#[cfg(unix)]
fn terminal_size() -> Option<(u16, u16)> {
    for fd in [libc::STDOUT_FILENO, libc::STDIN_FILENO] {
        let mut ws = libc::winsize {
            ws_row: 0,
            ws_col: 0,
            ws_xpixel: 0,
            ws_ypixel: 0,
        };
        // SAFETY: valid winsize out-pointer for TIOCGWINSZ.
        if unsafe { libc::ioctl(fd, libc::TIOCGWINSZ, &mut ws) } == 0
            && ws.ws_col > 0
            && ws.ws_row > 0
        {
            return Some((ws.ws_col, ws.ws_row));
        }
    }
    None
}

/// Windows: the size follows `--size` only.
#[cfg(not(unix))]
fn terminal_size() -> Option<(u16, u16)> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn feed(decoder: &mut StreamDecoder, lines: &[&str]) -> Vec<StreamEvent> {
        lines.iter().filter_map(|l| decoder.on_line(l)).collect()
    }

    #[test]
    fn output_before_the_seed_reply_is_dropped_and_after_it_is_delivered() {
        let mut decoder = StreamDecoder::default();
        let events = feed(
            &mut decoder,
            &[
                "%output %3 early",
                "%begin 1 1 1",
                "pane %3 80x24",
                "\x1b[Hseed\r",
                "line two",
                "%end 1 1 1",
                "%output %3 late\\015\\012",
                "%pane-exited %3 7",
                "%pane-respawned %3",
                "%exit",
            ],
        );
        assert_eq!(
            events,
            vec![
                StreamEvent::Seed {
                    pane: "%3".to_string(),
                    size: (80, 24),
                    exited: None,
                    seed: b"\x1b[Hseed\r\nline two".to_vec(),
                },
                StreamEvent::Output(b"late\r\n".to_vec()),
                StreamEvent::Exited(Some(7)),
                StreamEvent::Respawned,
                StreamEvent::Closed,
            ]
        );
    }

    #[test]
    fn a_refused_stream_is_an_error_reply_and_a_held_pane_reports_its_code() {
        let mut refused = StreamDecoder::default();
        assert_eq!(
            feed(
                &mut refused,
                &[
                    "%begin 1 1 1",
                    "observe-pane: no such pane: %9",
                    "%error 1 1 1"
                ]
            ),
            vec![StreamEvent::Reply {
                ok: false,
                body: vec!["observe-pane: no such pane: %9".to_string()],
            }]
        );
        let mut held = StreamDecoder::default();
        let events = feed(
            &mut held,
            &["%begin 1 1 1", "pane %0 40x10 exited=?", "%end 1 1 1"],
        );
        assert!(matches!(
            &events[0],
            StreamEvent::Seed {
                exited: Some(None),
                size: (40, 10),
                ..
            }
        ));
    }
}
