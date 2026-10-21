#!/bin/bash
# Cap Par-mux.app daemon logs in place (pattern proven on par-canvas/herdr:
# vault cap-launchd-standardoutpath-log-in-place). launchd and the app launcher
# hold their log fds open with O_APPEND, so truncate-in-place is safe and rename
# rotation is NOT — a rename strands subsequent writes in the moved inode.
# Over the cap: copy -> truncate -> gzip the copy. Best-effort: never fatal.
# Test overrides: PAR_MUX_LOG_DIR, PAR_MUX_LOG_CAP_BYTES.
set -u
log_dir=${PAR_MUX_LOG_DIR:-"$HOME/Library/Logs/par-mux"}
cap_bytes=${PAR_MUX_LOG_CAP_BYTES:-52428800}
for f in "$log_dir/daemon.log" "$log_dir/launchd.err.log" "$log_dir/launchd.out.log"; do
  [ -f "$f" ] || continue
  size=$(stat -f '%z' "$f" 2>/dev/null) || continue
  [ "$size" -le "$cap_bytes" ] && continue
  cp "$f" "$f.cap.tmp" && : > "$f" && gzip -f "$f.cap.tmp" && mv -f "$f.cap.tmp.gz" "$f.1.gz" || {
    rm -f "$f.cap.tmp" "$f.cap.tmp.gz"
    continue
  }
done
exit 0
