//! The management chord defaults vs the docs/MUX.md settings row
//! (card 01a16c62788b714f814ed982dee531a5).

use crate::mux::config::*;

/// The settings table's chord row in docs/MUX.md enumerates every
/// management chord and its default, in the doc's order, and each
/// documented default is Management::default's byte — either side
/// drifting alone fails here (card 01a16c62788b714f814ed982dee531a5).
#[test]
fn the_settings_doc_row_enumerates_every_management_default() {
    let doc = std::fs::read_to_string(concat!(env!("CARGO_MANIFEST_DIR"), "/../../docs/MUX.md"))
        .expect("docs/MUX.md is readable from the crate");
    let line = doc
        .lines()
        .find(|l| l.contains("`client.split-right` / `split-down`"))
        .expect("the settings table's chord row");
    let documented: Vec<&str> = line
        .split("defaults `")
        .nth(1)
        .expect("the row carries a defaults enumeration")
        .split("`)")
        .next()
        .expect("the enumeration closes")
        .split("`, `")
        .map(str::trim)
        .collect();
    let m = Management::default();
    // (doc key, documented default spelling, actual default byte), in
    // the doc row's order.
    let expected = [
        ("split-right", "%", m.split_right, b'%'),
        ("split-down", "\"", m.split_down, b'"'),
        ("kill-pane", "x", m.kill_pane, b'x'),
        ("break-pane", "!", m.break_pane, b'!'),
        ("join-pane", "J", m.join_pane, b'J'),
        ("palette", ":", m.palette, b':'),
        ("new-window", "c", m.new_window, b'c'),
        ("swap-prev", "{", m.swap_prev, b'{'),
        ("swap-next", "}", m.swap_next, b'}'),
        ("move-window-left", "<", m.move_window_left, b'<'),
        ("move-window-right", ">", m.move_window_right, b'>'),
        ("tab-strip-left", "H", m.tab_strip_left, b'H'),
        ("tab-strip-right", "L", m.tab_strip_right, b'L'),
        ("workspace-next", "W", m.workspace_next, b'W'),
        ("workspace-prev", "C-w", m.workspace_prev, 0x17),
        ("resize", "R", m.resize, b'R'),
        ("help", "?", m.help, b'?'),
        ("picker", "w", m.picker, b'w'),
        ("zoom", "z", m.zoom, b'z'),
        ("rename-window", ",", m.rename_window, b','),
        ("rename-pane", "$", m.rename_pane, b'$'),
        ("border-cycle", "B", m.border_cycle, b'B'),
        ("label-toggle", "l", m.label_toggle, b'l'),
        ("workspace-picker", "g", m.workspace_picker, b'g'),
        ("sidebar", "s", m.sidebar, b's'),
        ("status-bar", "S", m.status_bar, b'S'),
        ("respawn", "r", m.respawn, b'r'),
        ("notify-jump", "j", m.notify_jump, b'j'),
    ];
    let spellings: Vec<String> = expected
        .iter()
        .map(|(key, doc, field, byte)| {
            assert_eq!(field, byte, "{key}'s documented default is wrong");
            (*doc).to_string()
        })
        .collect();
    assert_eq!(
        documented, spellings,
        "the doc row's defaults must match Management::default one-for-one"
    );
    for (key, _, _, _) in &expected {
        let bare = format!(" / `{key}`");
        assert!(
            line.contains(&format!("`client.{key}`")) || line.contains(&bare),
            "the doc row lists the {key} chord"
        );
    }
}
