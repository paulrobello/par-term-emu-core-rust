//! Control-line plumbing: bounded line reads, the read outcome enum,
//! log summaries, eviction-aware writes, and reply classification.

use super::*;

/// How one bounded fill attempt ended (SEC-127).
#[derive(Debug, PartialEq, Eq)]
pub(super) enum LineFill {
    /// `buf` ends in `\n`.
    Complete,
    /// `buf` grew past `max`; the caller answers %error and closes.
    Oversize,
    /// EOF; `buf` holds any unterminated final line.
    Eof,
}

/// Append bytes from `reader` to `buf` until a newline, EOF, or the budget
/// trips. Errors (a recv-timeout poll wake above all) propagate with `buf`
/// intact: the bytes stay raw until the line completes, so a wake that
/// splits a multi-byte UTF-8 char loses nothing.
pub(super) fn fill_line_bounded<R: BufRead>(
    reader: &mut R,
    buf: &mut Vec<u8>,
    max: usize,
) -> std::io::Result<LineFill> {
    loop {
        let chunk = match reader.fill_buf() {
            Ok(chunk) => chunk,
            Err(err) if err.kind() == std::io::ErrorKind::Interrupted => continue,
            Err(err) => return Err(err),
        };
        if chunk.is_empty() {
            return Ok(LineFill::Eof);
        }
        // max + 1 - len >= 1 while len <= max, and the Oversize return below
        // stops the loop the moment len exceeds max.
        let budget = max + 1 - buf.len();
        let newline = chunk.iter().position(|&b| b == b'\n');
        let take = newline.map_or(chunk.len(), |i| i + 1).min(budget);
        buf.extend_from_slice(&chunk[..take]);
        reader.consume(take);
        if buf.len() > max {
            return Ok(LineFill::Oversize);
        }
        if buf.last() == Some(&b'\n') {
            return Ok(LineFill::Complete);
        }
    }
}

/// What [`read_control_line`] produced for one line of a connection.
#[derive(Debug, PartialEq, Eq)]
pub(super) enum ControlLine {
    /// A complete line (still carrying its `\n` when it had one), or an
    /// unterminated final line read before EOF.
    Line(String),
    /// The line passed the budget; the caller answers %error and closes.
    Oversize,
    /// A line that is not UTF-8, read through its newline so framing
    /// survives; carries its byte length.
    Undecodable(usize),
    /// EOF with nothing pending, a read fault, or eviction: stop serving.
    Closed,
}

/// Read one control line (SEC-127's bounded accumulation). Bytes stay raw
/// across recv-timeout wakes — `read_line` would discard a partial whose
/// tail splits a multi-byte char — so a healthy sender's pause costs
/// nothing and only an evicted connection ends; the budget is checked per
/// chunk, so an unterminated stream trips it without a newline; UTF-8 is
/// decoded once, after the line is complete. An unterminated final line is
/// returned before EOF, matching `Lines`' last-item behavior.
pub(super) fn read_control_line<R: BufRead>(reader: &mut R, evicted: &AtomicBool) -> ControlLine {
    let mut buf: Vec<u8> = Vec::new();
    loop {
        match fill_line_bounded(reader, &mut buf, MAX_CONTROL_LINE_BYTES) {
            Ok(LineFill::Eof) if buf.is_empty() => return ControlLine::Closed,
            Ok(LineFill::Eof) => break,
            Ok(LineFill::Oversize) => return ControlLine::Oversize,
            Ok(LineFill::Complete) => break,
            Err(err) if is_poll_wake(&err) => {
                if evicted.load(Ordering::Relaxed) {
                    return ControlLine::Closed;
                }
            }
            Err(_) => return ControlLine::Closed,
        }
    }
    match String::from_utf8(buf) {
        Ok(line) => ControlLine::Line(line),
        Err(err) => ControlLine::Undecodable(err.as_bytes().len()),
    }
}

/// True when a dispatch reply block ends in `%error` — the daemon's
/// rejection shape ([`emit_block`] with `ok: false`).
pub(super) fn reply_is_error(reply: &str) -> bool {
    reply
        .lines()
        .last()
        .is_some_and(|l| l.starts_with("%error"))
}

/// Commands whose arguments carry user data (typed input, clipboard):
/// logged as name, target and size only (SEC-132).
pub(super) const PAYLOAD_COMMANDS: &[&str] = &["send-keys", "set-buffer"];

/// A command line reduced for logging. A [`PAYLOAD_COMMANDS`] line keeps
/// only its name, a leading `-t` target and its size, so it is safe to log
/// whether or not it parses; any other line keeps its head and size, cut on
/// a char boundary.
pub(super) fn summarize_line(line: &str) -> String {
    const HEAD: usize = 120;
    let mut tokens = line.split_whitespace();
    if let Some(name) = tokens.next().filter(|name| PAYLOAD_COMMANDS.contains(name)) {
        // The target is read only from the leading flag, so a `-t` quoted
        // inside a payload can never pull a payload token into the log.
        let target = match (tokens.next(), tokens.next()) {
            (Some("-t"), Some(value)) => format!(" -t {value}"),
            _ => String::new(),
        };
        return format!("{name}{target} [payload redacted, {} bytes]", line.len());
    }
    if line.len() <= HEAD {
        return line.to_string();
    }
    let cut = line
        .char_indices()
        .take_while(|(i, _)| *i < HEAD)
        .map(|(i, _)| i)
        .last()
        .unwrap_or(0);
    format!("{}... ({} bytes total)", &line[..cut], line.len())
}

/// Write one line, tolerating send-timeout wakes: a healthy slow consumer's
/// buffer-full pause retries the remaining bytes, while an evicted client's
/// pause abandons the line — the connection is being torn down.
pub(super) fn write_line(
    writer: &mut LocalStream,
    line: &str,
    evicted: &AtomicBool,
) -> std::io::Result<()> {
    let mut buf = line.as_bytes();
    loop {
        match writer.write(buf) {
            Ok(0) => {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::WriteZero,
                    "the client socket wrote zero bytes",
                ));
            }
            Ok(n) => {
                buf = &buf[n..];
                if buf.is_empty() {
                    return writer.flush();
                }
            }
            Err(err) if is_poll_wake(&err) => {
                if evicted.load(Ordering::Relaxed) {
                    return Err(err);
                }
            }
            Err(err) => return Err(err),
        }
    }
}

/// Whether an I/O error is a send/recv-timeout wake rather than a fault —
/// the timeout-driven poll loop's transient, to be retried or examined.
pub(super) fn is_poll_wake(err: &std::io::Error) -> bool {
    matches!(
        err.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
    )
}
