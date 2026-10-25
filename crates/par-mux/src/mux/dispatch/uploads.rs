//! Upload handlers: `upload-begin`/`upload-chunk`/`upload-commit`/
//! `upload-abort`. Staging and its bounds live in
//! [`crate::mux::uploads`]; this file is the dispatch half — the reply
//! bodies (`upload-begin` answers the id, `upload-commit` answers the
//! materialized path) and the paste at commit.

use super::*;
use crate::mux::uploads;

/// The upload group's router; `route_command` sends only this group's
/// variants here.
pub(super) fn route_upload_command(ctx: &Ctx<'_>, command: MuxCommand) -> Outcome {
    match command {
        MuxCommand::UploadBegin {
            filename,
            total_bytes,
        } => cmd_upload_begin(ctx, &filename, total_bytes),
        MuxCommand::UploadChunk { upload, data } => cmd_upload_chunk(ctx, upload, &data),
        MuxCommand::UploadCommit { upload, pane } => cmd_upload_commit(ctx, upload, pane),
        MuxCommand::UploadAbort { upload } => cmd_upload_abort(ctx, upload),
        other => unreachable!("route_command sent a non-upload command: {other:?}"),
    }
}

fn cmd_upload_begin(ctx: &Ctx<'_>, filename: &str, total_bytes: u64) -> Outcome {
    match uploads::begin(filename, total_bytes) {
        Ok(id) => Outcome::ok(ctx, &id.to_string()),
        Err(err) => Outcome::err(ctx, &err),
    }
}

fn cmd_upload_chunk(ctx: &Ctx<'_>, upload: u64, data: &[u8]) -> Outcome {
    match uploads::chunk(upload, data) {
        Ok(()) => Outcome::ok(ctx, ""),
        Err(err) => Outcome::err(ctx, &err),
    }
}

fn cmd_upload_abort(ctx: &Ctx<'_>, upload: u64) -> Outcome {
    match uploads::abort(upload) {
        Ok(()) => Outcome::ok(ctx, ""),
        Err(err) => Outcome::err(ctx, &err),
    }
}

/// `upload-commit -t <pane> <id>`: seal the spool file, then bracket-paste
/// its PATH into the pane — the documented input semantic (the daemon
/// never injects the bytes themselves into pane input; the pane app reads
/// the file it is pointed at). The pane is resolved and its paste mode
/// read under the tree lock, then the write happens with the lock
/// released (the QA-225 snapshot-then-write shape).
///
/// Commit consumes the upload either way: a paste that fails leaves the
/// materialized file in place and says where, so the bytes are never
/// silently lost — the client can retry the paste or clean up.
fn cmd_upload_commit(ctx: &Ctx<'_>, upload: u64, pane: Target<PaneId>) -> Outcome {
    let path = match uploads::commit(upload) {
        Ok(path) => path,
        Err(err) => return Outcome::err(ctx, &err),
    };
    let remains = format!("upload-commit: the file remains at {}", path.display());
    let (input, bracketed) = {
        let guard = ctx.tree.lock();
        let pane_id = match guard.resolve_pane_target(pane) {
            Ok(id) => id,
            Err(err) => {
                return Outcome::err(ctx, &format!("upload-commit: {err}; {remains}"));
            }
        };
        let Some(target) = guard.pane(pane_id) else {
            return Outcome::err(
                ctx,
                &format!("upload-commit: no such pane: {pane_id}; {remains}"),
            );
        };
        let input = match target.input_handle() {
            Ok(handle) => handle,
            Err(err) => {
                return Outcome::err(ctx, &format!("upload-commit: {err}; {remains}"));
            }
        };
        let bracketed = target.terminal().read().bracketed_paste();
        (input, bracketed)
    };
    match input.write(&paste_body(&path.to_string_lossy(), bracketed)) {
        Ok(()) => Outcome::ok(ctx, &path.to_string_lossy()),
        Err(err) => Outcome::err(ctx, &format!("upload-commit: {err}; {remains}")),
    }
}

/// The bytes a commit writes to the pane: the path, framed by the pane's
/// own bracketed-paste markers when it has bracketed paste on (the same
/// framing `agent-prompt` uses), so a TUI receiving it sees one paste and
/// never a keystroke stream. A stray terminator inside the path is
/// stripped defensively, as `agent-prompt` strips it from prompt text.
fn paste_body(path: &str, bracketed: bool) -> Vec<u8> {
    let mut body = Vec::with_capacity(path.len() + 8);
    if bracketed {
        body.extend_from_slice(b"\x1b[200~");
    }
    body.extend_from_slice(path.replace("\x1b[201~", "").as_bytes());
    if bracketed {
        body.extend_from_slice(b"\x1b[201~");
    }
    body
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn paste_body_frames_only_when_the_pane_has_bracketed_paste_on() {
        let path = "/tmp/par-mux-uploads/upload-000001-photo.png";
        assert_eq!(
            paste_body(path, false),
            path.as_bytes().to_vec(),
            "plain mode: the bare path"
        );
        assert_eq!(
            paste_body(path, true),
            format!("\x1b[200~{path}\x1b[201~").into_bytes(),
            "bracketed mode: the pane's own markers frame the path"
        );
        // An embedded terminator is stripped so it cannot end the paste
        // early.
        assert_eq!(
            paste_body("/tmp/a\x1b[201~b", true),
            b"\x1b[200~/tmp/ab\x1b[201~".to_vec(),
        );
    }
}
