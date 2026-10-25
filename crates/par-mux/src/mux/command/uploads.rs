//! Upload command parsers: `upload-begin`, `upload-chunk`,
//! `upload-commit`, `upload-abort` — the control verbs a client (the
//! phone, over the mux-control channel) drives to send a file into a
//! pane. The staging bounds live in [`crate::mux::uploads`]; these
//! parsers only shape the wire.

use super::*;
use base64::Engine as _;

/// `upload-begin <filename> <total-bytes>`: two shell words. The filename
/// is a bare file name (a path prefix is stripped daemon-side), quoted
/// when it carries spaces, and the total is the byte count the chunks
/// must land exactly.
pub(super) fn parse_upload_begin(a: &Args<'_>) -> Result<MuxCommand, String> {
    let lead = split_leading_flags(a.line, a.name, &[])?;
    let mut words = shell_split(lead.tail).into_iter();
    let Some(filename) = words.next() else {
        return Err(format!(
            "{} requires a filename and a total byte count",
            a.name
        ));
    };
    let Some(total) = words.next() else {
        return Err(format!("{} requires a total byte count", a.name));
    };
    if let Some(extra) = words.next() {
        return Err(format!("{}: unexpected argument {extra:?}", a.name));
    }
    let total_bytes = total
        .parse::<u64>()
        .map_err(|_| format!("{}: invalid total bytes: {total}", a.name))?;
    Ok(MuxCommand::UploadBegin {
        filename,
        total_bytes,
    })
}

/// `upload-chunk <id> <base64>`: the chunk rides as one standard-alphabet
/// base64 token, the byte-exact escape hatch for binary payloads on the
/// line-framed control wire (the same role `-H` plays for `set-buffer`).
pub(super) fn parse_upload_chunk(a: &Args<'_>) -> Result<MuxCommand, String> {
    let lead = split_leading_flags(a.line, a.name, &[])?;
    let mut words = shell_split(lead.tail).into_iter();
    let Some(id) = words.next() else {
        return Err(format!(
            "{} requires an upload id and a base64 chunk",
            a.name
        ));
    };
    let Some(payload) = words.next() else {
        return Err(format!("{} requires a base64 chunk", a.name));
    };
    if let Some(extra) = words.next() {
        return Err(format!("{}: unexpected argument {extra:?}", a.name));
    }
    let upload = id
        .parse::<u64>()
        .map_err(|_| format!("{}: invalid upload id: {id}", a.name))?;
    let data = base64::engine::general_purpose::STANDARD
        .decode(payload.as_bytes())
        .map_err(|_| format!("{}: chunk is not valid base64", a.name))?;
    Ok(MuxCommand::UploadChunk { upload, data })
}

/// `upload-commit -t <pane> <id>`: the pane is chosen at commit — the
/// same `-t <pane>` addressing every other content verb uses.
pub(super) fn parse_upload_commit(a: &Args<'_>) -> Result<MuxCommand, String> {
    use LeadingFlag::Valued;
    let lead = split_leading_flags(a.line, a.name, &[Valued("-t")])?;
    let pane = pane_flag(a.name, lead.value("-t"))?;
    let mut words = shell_split(lead.tail).into_iter();
    let Some(id) = words.next() else {
        return Err(format!("{} requires an upload id", a.name));
    };
    if let Some(extra) = words.next() {
        return Err(format!("{}: unexpected argument {extra:?}", a.name));
    }
    let upload = id
        .parse::<u64>()
        .map_err(|_| format!("{}: invalid upload id: {id}", a.name))?;
    Ok(MuxCommand::UploadCommit { upload, pane })
}

/// `upload-abort <id>`.
pub(super) fn parse_upload_abort(a: &Args<'_>) -> Result<MuxCommand, String> {
    let lead = split_leading_flags(a.line, a.name, &[])?;
    let mut words = shell_split(lead.tail).into_iter();
    let Some(id) = words.next() else {
        return Err(format!("{} requires an upload id", a.name));
    };
    if let Some(extra) = words.next() {
        return Err(format!("{}: unexpected argument {extra:?}", a.name));
    }
    let upload = id
        .parse::<u64>()
        .map_err(|_| format!("{}: invalid upload id: {id}", a.name))?;
    Ok(MuxCommand::UploadAbort { upload })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(line: &str) -> Result<MuxCommand, String> {
        parse_command(line)
    }

    #[test]
    fn upload_begin_parses_name_and_total() {
        let command = parse("upload-begin photo.png 12345").expect("parse");
        assert_eq!(
            command,
            MuxCommand::UploadBegin {
                filename: "photo.png".to_string(),
                total_bytes: 12345,
            }
        );
        // A quoted filename carries spaces (the same bounded grammar the
        // set-buffer payload uses).
        assert_eq!(
            parse("upload-begin 'my photo.png' 0").expect("parse"),
            MuxCommand::UploadBegin {
                filename: "my photo.png".to_string(),
                total_bytes: 0,
            }
        );
    }

    #[test]
    fn upload_begin_shape_errors() {
        assert!(parse("upload-begin").is_err());
        assert!(parse("upload-begin only-name").is_err());
        assert!(parse("upload-begin a.png 12 extra").is_err());
        assert!(parse("upload-begin a.png NaN").is_err());
    }

    #[test]
    fn upload_chunk_decodes_base64() {
        assert_eq!(
            parse("upload-chunk 7 aGVsbG8=").expect("parse"),
            MuxCommand::UploadChunk {
                upload: 7,
                data: b"hello".to_vec(),
            }
        );
    }

    #[test]
    fn upload_chunk_shape_errors() {
        assert!(parse("upload-chunk").is_err());
        assert!(parse("upload-chunk 7").is_err());
        assert!(parse("upload-chunk x aGVsbG8=").is_err());
        assert!(parse("upload-chunk 7 !!!not-base64!!!").is_err());
    }

    #[test]
    fn upload_commit_parses_pane_and_id() {
        assert_eq!(
            parse("upload-commit -t %3 42").expect("parse"),
            MuxCommand::UploadCommit {
                upload: 42,
                pane: Target::Id(PaneId(3)),
            }
        );
        assert!(parse("upload-commit 42").is_err(), "-t is required");
        assert!(parse("upload-commit -t %3").is_err(), "id is required");
        assert!(parse("upload-commit -t %3 NaN").is_err());
    }

    #[test]
    fn upload_abort_parses_the_id() {
        assert_eq!(
            parse("upload-abort 9").expect("parse"),
            MuxCommand::UploadAbort { upload: 9 }
        );
        assert!(parse("upload-abort").is_err());
        assert!(parse("upload-abort 9 8").is_err());
    }
}
