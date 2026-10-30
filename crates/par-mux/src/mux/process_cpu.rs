//! Cumulative process-group CPU for a pane's child process tree — the
//! data behind pane-info's `cpu_ms=` token.
//!
//! The pane's process group is walked as the OS parent-link tree:
//! portable-pty seats the child in its own process group/session on Unix,
//! so its live descendants carry its pgid, and the walk finds them without
//! a per-platform pgid API. Grandchildren a double-fork reparented to
//! init are missed. On Windows the same walk applies, and ConPTY's
//! conhost stays uncounted — it hosts the shell, it is not its child.
//! The whole OS process table is read per call, so the pane-info reply
//! takes it OFF the tree lock — the same discipline as the foreground
//! command's `ProcessTable` snapshot.

use std::collections::HashMap;

use sysinfo::{Pid, ProcessesToUpdate, System};

/// Cumulative CPU milliseconds across `root`'s process subtree (root
/// included), or `None` when `root` is absent from the process table (a
/// reaped or unknown pid). sysinfo's `accumulated_cpu_time` is already
/// CPU-milliseconds and may exceed wall time on a multi-core host.
pub(crate) fn group_cpu_ms(root: u32) -> Option<u64> {
    let mut system = System::new();
    system.refresh_processes(ProcessesToUpdate::All, true);
    let processes = system.processes();
    let root_pid = Pid::from_u32(root);
    if !processes.contains_key(&root_pid) {
        return None;
    }
    let mut children: HashMap<Pid, Vec<Pid>> = HashMap::new();
    for (pid, process) in processes {
        if let Some(parent) = process.parent() {
            children.entry(parent).or_default().push(*pid);
        }
    }
    let mut total = 0u64;
    let mut stack = vec![root_pid];
    while let Some(pid) = stack.pop() {
        if let Some(process) = processes.get(&pid) {
            total = total.saturating_add(process.accumulated_cpu_time());
            if let Some(kids) = children.get(&pid) {
                stack.extend_from_slice(kids);
            }
        }
    }
    Some(total)
}

#[cfg(test)]
mod tests {
    use super::group_cpu_ms;

    /// The test process is in the OS table, and by the time a test runs
    /// the binary's runtime init has burned measurable CPU.
    #[test]
    fn own_process_reports_cumulative_cpu() {
        let ms = group_cpu_ms(std::process::id()).expect("the test process is in the table");
        assert!(ms > 0, "a running test binary has burned CPU: {ms}");
    }

    /// A pid absent from the process table reads as unknown — no token,
    /// never a zero.
    #[test]
    fn an_absent_pid_reports_none() {
        assert_eq!(group_cpu_ms(u32::MAX - 1), None);
    }

    /// A real child burning CPU shows up in its own subtree sum — the
    /// accounting behind a pane's `cpu_ms` (cfg(unix): spawns `sh`).
    #[cfg(unix)]
    #[test]
    fn a_burning_child_accumulates_cpu() {
        let mut child = std::process::Command::new("sh")
            .arg("-c")
            .arg("i=0; while [ $i -lt 2000000 ]; do i=$((i+1)); done; sleep 5")
            .spawn()
            .expect("sh spawns");
        // Bounded poll: the child's accumulated CPU crosses zero well
        // inside its burn loop (2M arithmetic iterations).
        let mut saw = None;
        for _ in 0..100 {
            if let Some(ms) = group_cpu_ms(child.id()) {
                if ms > 0 {
                    saw = Some(ms);
                    break;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(50));
        }
        child.kill().ok();
        child.wait().ok();
        let ms = saw.expect("a burning child accumulates CPU");
        assert!(ms > 0, "the burn landed: {ms}");
    }
}
