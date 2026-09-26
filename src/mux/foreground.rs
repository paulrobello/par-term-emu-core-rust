//! Liveness probing for hook-authoritative agent claims.
//!
//! `pane.release_agent` clears a claim only when the agent's hook SENDS it,
//! so an agent that crashes or is killed would keep its roster entry and
//! resume identity until a relabel or pane death. The scrape tick closes
//! that gap (card 01a0da3afac47d12afeb521f6878c0ae): a hook claim is stale
//! when no process descending from the pane's child matches the claimed
//! agent's CLI.
//!
//! Descendant scan rather than herdr's foreground-process check, because
//! the foreground answer is wrong in both directions for our question: an
//! agent backgrounded with Ctrl+Z or hidden behind an editor the user
//! spawned is still alive, and only absence from the pane's process tree
//! proves death. The verdict is conservative by design —
//! [`Liveness::Unknown`] keeps the claim, and the matcher biases toward
//! matching (a false keep delays the clear by a tick; a false clear drops
//! a live agent's identity and resume chain).
//!
//! Process enumeration uses plain `sysctl`/`/proc`, never libproc:
//! `proc_listallpids`/`proc_pidinfo` are gated on modern macOS to the
//! caller's own ancestry (measured 2026-09-25: EPERM for every pid
//! outside it, collapsing the table to six processes), which would blind
//! the sweep to exactly the panes it exists for.
//!
//! Windows has no portable process-tree/argv access in this crate's deps,
//! so [`ProcessTable::snapshot`] is `None` there and hook release stays the
//! only claim-clearing path.

/// What a liveness probe proved about a pane's claimed agent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Liveness {
    /// A descendant of the pane's child matches the claimed agent's CLI.
    Matches,
    /// The pane's whole descendant tree was enumerable, every argv was
    /// readable, and none matched — the agent provably died.
    Mismatch,
    /// The probe could not prove either way (table unavailable, root
    /// missing, or a descendant's argv unreadable). Keeps the claim.
    Unknown,
}

/// One process in the table: identity always known, argv only when stated.
struct ProcMeta {
    pid: i32,
    ppid: i32,
}

/// A snapshot of the process table, answerable for one pane's liveness.
///
/// Real tables ([`Self::snapshot`]) carry pid/ppid for every process and
/// resolve a candidate's argv from the OS on demand — few pids per pane,
/// and an uncached read is more correct than a cached one (an `exec` keeps
/// the pid but replaces the argv). Test tables ([`Self::with_fixed_argv`])
/// answer argv from a fixed map and never touch the OS.
pub(crate) struct ProcessTable {
    metas: Vec<ProcMeta>,
    /// Test seam: when set, argv lookups consult this map instead of the
    /// OS. A present `None` is an unreadable argv; a missing pid is too.
    fixed_argv: Option<std::collections::HashMap<i32, Option<Vec<String>>>>,
}

impl ProcessTable {
    /// The real process table, or `None` when it cannot be read (Windows:
    /// always — no portable tree/argv access; see the module docs).
    #[cfg(unix)]
    pub(crate) fn snapshot() -> Option<ProcessTable> {
        Some(ProcessTable {
            metas: snapshot_metas()?,
            fixed_argv: None,
        })
    }

    #[cfg(windows)]
    pub(crate) fn snapshot() -> Option<ProcessTable> {
        None
    }

    /// A test table: `(pid, ppid, argv)` triples where `argv = None` means
    /// "present but unreadable" — the probe must report [`Liveness::Unknown`]
    /// rather than guess.
    #[cfg(test)]
    pub(crate) fn with_fixed_argv(entries: &[(i32, i32, Option<Vec<String>>)]) -> ProcessTable {
        ProcessTable {
            metas: entries
                .iter()
                .map(|(pid, ppid, _)| ProcMeta {
                    pid: *pid,
                    ppid: *ppid,
                })
                .collect(),
            fixed_argv: Some(
                entries
                    .iter()
                    .map(|(pid, _, argv)| (*pid, argv.clone()))
                    .collect(),
            ),
        }
    }

    /// Whether any process in `root`'s descendant tree (root included)
    /// matches the claimed agent `label`.
    pub(crate) fn agent_alive(&self, root: u32, label: &str) -> Liveness {
        let root = root as i32;
        if !self.metas.iter().any(|meta| meta.pid == root) {
            // The pane's child is not in the table — a race with process
            // exit at worst; the pane's own running flag is the authority
            // for that, so this is Unknown, not a mismatch.
            return Liveness::Unknown;
        }
        let mut children: std::collections::HashMap<i32, Vec<i32>> =
            std::collections::HashMap::new();
        for meta in &self.metas {
            children.entry(meta.ppid).or_default().push(meta.pid);
        }
        let mut stack = vec![root];
        let mut visited = std::collections::HashSet::new();
        let mut unprovable = false;
        while let Some(pid) = stack.pop() {
            if !visited.insert(pid) {
                continue;
            }
            match self.argv_of(pid) {
                Some(argv) if argv_matches(&argv, label) => return Liveness::Matches,
                Some(_) => {}
                None => unprovable = true,
            }
            if let Some(kids) = children.get(&pid) {
                stack.extend(kids.iter().copied());
            }
        }
        if unprovable {
            Liveness::Unknown
        } else {
            Liveness::Mismatch
        }
    }

    fn argv_of(&self, pid: i32) -> Option<Vec<String>> {
        match &self.fixed_argv {
            Some(map) => map.get(&pid).cloned().flatten(),
            None => read_argv(pid),
        }
    }
}

/// Whether an argv plausibly belongs to the claimed agent's CLI.
///
/// Agents are npm/bun-installed, so the label may appear as the direct
/// binary (`pi`), a path component (`/…/.bun/bin/pi`), or inside a
/// package path the interpreter executes (`node …/claude-cli/cli.js`) —
/// the component-suffix rules cover those shapes. The bias is toward
/// matching: a stray component like `pi-notes.txt` in some other
/// command's argv keeps the claim one more tick, which only delays the
/// clear; failing to match a live agent's CLI would drop its identity.
pub(crate) fn argv_matches(argv: &[String], label: &str) -> bool {
    if label.is_empty() {
        return false;
    }
    let dash = format!("{label}-");
    let underscore = format!("{label}_");
    let dot = format!("{label}.");
    argv.iter().any(|arg| {
        arg.split('/').any(|component| {
            component == label
                || component.starts_with(&dash)
                || component.starts_with(&underscore)
                || component.starts_with(&dot)
        })
    })
}

// ---- Linux: /proc ----

#[cfg(target_os = "linux")]
fn snapshot_metas() -> Option<Vec<ProcMeta>> {
    let mut metas = Vec::new();
    let entries = std::fs::read_dir("/proc").ok()?;
    for entry in entries.flatten() {
        let name = entry.file_name();
        let Some(pid) = name.to_str().and_then(|s| s.parse::<i32>().ok()) else {
            continue;
        };
        let Ok(stat) = std::fs::read_to_string(entry.path().join("stat")) else {
            continue;
        };
        // "pid (comm) state ppid …" — comm may contain spaces and parens,
        // so parse from the LAST ')' rather than splitting the whole line.
        let Some((_, rest)) = stat.rsplit_once(')') else {
            continue;
        };
        let mut fields = rest.split_whitespace();
        let _state = fields.next();
        let Some(ppid) = fields.next().and_then(|f| f.parse::<i32>().ok()) else {
            continue;
        };
        metas.push(ProcMeta { pid, ppid });
    }
    Some(metas)
}

#[cfg(target_os = "linux")]
fn read_argv(pid: i32) -> Option<Vec<String>> {
    let raw = std::fs::read(format!("/proc/{pid}/cmdline")).ok()?;
    let argv: Vec<String> = raw
        .split(|&b| b == 0)
        .filter(|slice| !slice.is_empty())
        .map(|slice| String::from_utf8_lossy(slice).into_owned())
        .collect();
    if argv.is_empty() {
        None
    } else {
        Some(argv)
    }
}

// ---- macOS: KERN_PROC_ALL via plain sysctl ----

/// `kinfo_proc`'s fixed byte offsets and entry size. libc does not export
/// the struct (hand-transcribing all of `eprocess`, with its embedded
/// `vmspace`, is stride-fragile), so the offsets are read positionally —
/// `kp_proc.p_pid` at 40 (extern_proc's simple prefix: union + two
/// pointers + int + char + pad) and `kp_eproc.e_ppid` at 560 — and the
/// whole layout assumption is VERIFIED at every process start by
/// [`kinfo_layout`]'s calibration entry: this process's own kinfo_proc
/// must carry its pid and its getppid() at exactly these offsets. A
/// mismatched kernel layout fails calibration and the sweep goes inert
/// rather than guessing.
#[cfg(target_os = "macos")]
const KP_PID_OFFSET: usize = 40;

#[cfg(target_os = "macos")]
const KP_PPID_OFFSET: usize = 560;

#[cfg(target_os = "macos")]
fn kinfo_layout() -> Option<usize> {
    use std::sync::OnceLock;
    static LAYOUT: OnceLock<Option<usize>> = OnceLock::new();
    *LAYOUT.get_or_init(|| {
        let me = std::process::id() as libc::c_int;
        let mut mib: [libc::c_int; 4] = [libc::CTL_KERN, libc::KERN_PROC, libc::KERN_PROC_PID, me];
        // The size call's length is an upper bound (3888 measured), the
        // fill's is the real one-entry size (648) — allocate the bound,
        // trust the fill.
        let mut len: libc::size_t = 0;
        let fill_len = unsafe {
            if libc::sysctl(
                mib.as_mut_ptr(),
                4,
                std::ptr::null_mut(),
                &mut len,
                std::ptr::null_mut(),
                0,
            ) != 0
                || len < KP_PPID_OFFSET + 4
            {
                return None;
            }
            let mut buf = vec![0u8; len];
            if libc::sysctl(
                mib.as_mut_ptr(),
                4,
                buf.as_mut_ptr().cast(),
                &mut len,
                std::ptr::null_mut(),
                0,
            ) != 0
                || len < KP_PPID_OFFSET + 4
            {
                return None;
            }
            let pid = i32::from_ne_bytes(buf[KP_PID_OFFSET..KP_PID_OFFSET + 4].try_into().unwrap());
            let ppid =
                i32::from_ne_bytes(buf[KP_PPID_OFFSET..KP_PPID_OFFSET + 4].try_into().unwrap());
            (pid == me && ppid == libc::getppid()).then_some(len)?
        };
        Some(fill_len)
    })
}

#[cfg(target_os = "macos")]
fn snapshot_metas() -> Option<Vec<ProcMeta>> {
    let stride = kinfo_layout()?;
    let mut mib: [libc::c_int; 3] = [libc::CTL_KERN, libc::KERN_PROC, libc::KERN_PROC_ALL];
    let mut len: libc::size_t = 0;
    unsafe {
        if libc::sysctl(
            mib.as_mut_ptr(),
            3,
            std::ptr::null_mut(),
            &mut len,
            std::ptr::null_mut(),
            0,
        ) != 0
            || len < stride
        {
            return None;
        }
        let mut buf = vec![0u8; len];
        if libc::sysctl(
            mib.as_mut_ptr(),
            3,
            buf.as_mut_ptr().cast(),
            &mut len,
            std::ptr::null_mut(),
            0,
        ) != 0
        {
            return None;
        }
        buf.truncate(len);
        let mut metas = Vec::with_capacity(buf.len() / stride);
        let mut offset = 0;
        while offset + stride <= buf.len() {
            let pid = i32::from_ne_bytes(
                buf[offset + KP_PID_OFFSET..offset + KP_PID_OFFSET + 4]
                    .try_into()
                    .unwrap(),
            );
            let ppid = i32::from_ne_bytes(
                buf[offset + KP_PPID_OFFSET..offset + KP_PPID_OFFSET + 4]
                    .try_into()
                    .unwrap(),
            );
            // pid 0 is the kernel slot the ALL listing leads with; the
            // calibration entry has already verified the layout, so a
            // non-process entry is skipped, not fatal.
            if pid > 0 {
                metas.push(ProcMeta { pid, ppid });
            }
            offset += stride;
        }
        Some(metas)
    }
}

/// A process's argv, or `None` when unreadable (permissions, zombie,
/// mid-`exec` — the fresh-spawn window where the kernel cannot yet serve
/// the args of an exec'ing process).
#[cfg(target_os = "macos")]
fn read_argv(pid: i32) -> Option<Vec<String>> {
    // KERN_PROCARGS2 lays out: native-endian argc, argv[0] (the executable
    // path), argv[1..], envp… — all NUL-separated strings. Keeping the
    // first argc strings after the count yields argv.
    let mut mib: [libc::c_int; 3] = [libc::CTL_KERN, libc::KERN_PROCARGS2, pid];
    let mut len: libc::size_t = 0;
    if unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            3,
            std::ptr::null_mut(),
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    } != 0
    {
        return None;
    }
    let mut buf: Vec<u8> = vec![0; len];
    if unsafe {
        libc::sysctl(
            mib.as_mut_ptr(),
            3,
            buf.as_mut_ptr().cast(),
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    } != 0
    {
        return None;
    }
    buf.truncate(len);
    if buf.len() < 4 {
        return None;
    }
    let argc = i32::from_ne_bytes([buf[0], buf[1], buf[2], buf[3]]) as usize;
    let mut parts: Vec<String> = buf[4..]
        .split(|&b| b == 0)
        .filter(|slice| !slice.is_empty())
        .map(|slice| String::from_utf8_lossy(slice).into_owned())
        .collect();
    if parts.len() > argc {
        parts.truncate(argc);
    }
    if parts.is_empty() {
        None
    } else {
        Some(parts)
    }
}

// ---- Other unix / Windows: inert ----

#[cfg(all(not(target_os = "macos"), not(target_os = "linux"), unix))]
fn snapshot_metas() -> Option<Vec<ProcMeta>> {
    None
}

#[cfg(all(not(target_os = "macos"), not(target_os = "linux"), unix))]
fn read_argv(_pid: i32) -> Option<Vec<String>> {
    None
}

// `snapshot()` is `None` on Windows so `argv_of` never runs there, but the
// module must still compile.
#[cfg(windows)]
fn read_argv(_pid: i32) -> Option<Vec<String>> {
    None
}

#[cfg(test)]
mod tests {
    use super::*;

    fn argv(parts: &[&str]) -> Option<Vec<String>> {
        Some(parts.iter().map(|p| p.to_string()).collect())
    }

    #[test]
    fn matcher_accepts_the_direct_cli_shapes() {
        // Direct argv[0], bare and as a resolved path.
        assert!(argv_matches(
            &argv(&["pi", "--session", "/tmp/x"]).unwrap(),
            "pi"
        ));
        assert!(argv_matches(
            &argv(&["/usr/local/bin/claude", "--resume", "id"]).unwrap(),
            "claude"
        ));
        // Interpreter-executed package: the label rides a path component
        // of a script the interpreter runs (`node …/claude-cli/cli.js`).
        assert!(argv_matches(
            &argv(&[
                "/usr/local/bin/node",
                "/usr/local/lib/node_modules/@anthropic-ai/claude-cli/cli.js",
                "--resume",
                "id"
            ])
            .unwrap(),
            "claude"
        ));
        // bun-installed script whose file name is `<label>.js`.
        assert!(argv_matches(
            &argv(&[
                "/opt/homebrew/bin/bun",
                "/Users/x/.bun/install/global/node_modules/pi.js"
            ])
            .unwrap(),
            "pi"
        ));
    }

    #[test]
    fn matcher_rejects_partial_names_and_foreign_commands() {
        // `piano` is not `pi`: the suffix rules require a separator.
        assert!(!argv_matches(&argv(&["piano", "--play"]).unwrap(), "pi"));
        assert!(!argv_matches(&argv(&["/bin/bash", "-l"]).unwrap(), "pi"));
        // A login shell's argv[0] carries the leading dash — the pane's
        // shell must not satisfy any agent's claim.
        assert!(!argv_matches(&argv(&["-bash"]).unwrap(), "bash"));
        assert!(!argv_matches(&argv(&["vim", "notes"]).unwrap(), "pi"));
        assert!(!argv_matches(&argv(&["sleep", "300"]).unwrap(), "pi"));
        assert!(!argv_matches(&argv(&["sleep"]).unwrap(), ""));
    }

    #[test]
    fn matcher_keeps_the_claim_for_label_suffixed_files() {
        // A `pi-notes.txt` component in some other command's argv keeps the
        // claim — the deliberate toward-matching bias: it only delays the
        // clear, while the opposite error would drop a live claim.
        assert!(argv_matches(
            &argv(&["vim", "/tmp/pi-notes.txt"]).unwrap(),
            "pi"
        ));
    }

    #[test]
    fn the_root_process_itself_can_be_the_agent() {
        let table = ProcessTable::with_fixed_argv(&[(4242, 1, argv(&["pi"]))]);
        assert_eq!(table.agent_alive(4242, "pi"), Liveness::Matches);
    }

    #[test]
    fn a_grandchild_agent_keeps_the_claim() {
        // The pane's child is the shell; the agent is its child.
        let table = ProcessTable::with_fixed_argv(&[
            (100, 1, argv(&["-zsh"])),
            (2100, 100, argv(&["node", "/x/y/claude-cli/cli.js"])),
        ]);
        assert_eq!(table.agent_alive(100, "claude"), Liveness::Matches);
    }

    #[test]
    fn no_matching_descendant_is_a_provable_mismatch() {
        let table = ProcessTable::with_fixed_argv(&[
            (100, 1, argv(&["-zsh"])),
            (2100, 100, argv(&["vim", "notes.txt"])),
        ]);
        assert_eq!(table.agent_alive(100, "pi"), Liveness::Mismatch);
    }

    #[test]
    fn an_unreadable_descendant_is_unknown_not_mismatch() {
        let table = ProcessTable::with_fixed_argv(&[
            (100, 1, argv(&["-zsh"])),
            (2100, 100, None), // present, argv unreadable
            (2101, 100, argv(&["sleep", "30"])),
        ]);
        assert_eq!(table.agent_alive(100, "pi"), Liveness::Unknown);
    }

    #[test]
    fn a_missing_root_is_unknown() {
        let table = ProcessTable::with_fixed_argv(&[(100, 1, argv(&["-zsh"]))]);
        assert_eq!(table.agent_alive(9999, "pi"), Liveness::Unknown);
        let empty = ProcessTable::with_fixed_argv(&[]);
        assert_eq!(empty.agent_alive(100, "pi"), Liveness::Unknown);
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn the_layout_calibration_sees_this_process() {
        // Positive control for the positional-offset assumption: the
        // calibration entry exists and the pid offset parses this pid.
        assert!(kinfo_layout().is_some(), "calibration succeeds");
    }

    #[cfg(unix)]
    #[test]
    fn the_real_table_sees_this_process_tree() {
        // Positive control on the real enumeration path: this test's own
        // process must be in the table with a readable argv, matched by its
        // own executable name. (The provable-miss direction is NOT asserted
        // here — under the parallel suite this process parents transient
        // pane children whose argv is unreadable mid-exec, which is exactly
        // the Unknown the design keeps; the scrape real-table test proves
        // the clear on a stable single-shell tree.)
        let table = ProcessTable::snapshot().expect("snapshot works on unix");
        let me = std::process::id();
        let exe = std::env::current_exe()
            .expect("current exe")
            .file_name()
            .and_then(|n| n.to_str())
            .expect("exe name")
            .to_string();
        assert_eq!(
            table.agent_alive(me, &exe),
            Liveness::Matches,
            "the process's own executable name matches its own label"
        );
    }
}
