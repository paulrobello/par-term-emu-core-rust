"""RIS (ESC c) keeps embedder configuration (audit ARC-100).

Every public set_* on Terminal is classified: a probe proving it survives
RIS, a Rust test that covers it (no comparable Python getter), or a VT-state
entry saying why RIS resets it. A new setter fails the classification test
until someone decides.
"""

from collections.abc import Callable

import pytest
from par_term_emu_core_rust import (
    AmbiguousWidth,
    NormalizationForm,
    Terminal,
    UnicodeVersion,
    WidthConfig,
)

RIS = b"\x1bc"

Probe = tuple[Callable[[Terminal], object], Callable[[Terminal], object]]


def _fill_commands(t: Terminal) -> int:
    for i in range(6):
        t.start_command_execution(f"c{i}")
        t.end_command_execution(0)
    return len(t.get_command_history())


def _fill_cwds(t: Terminal) -> int:
    for i in range(4):
        t.record_cwd_change(f"/tmp/d{i}")
    return len(t.get_cwd_changes())


def _width(t: Terminal) -> tuple[int, int]:
    cfg = t.width_config()
    return (int(cfg.unicode_version), int(cfg.ambiguous_width))


def _notification_threshold(t: Terminal) -> int:
    return t.get_notification_config().activity_threshold


def _set_notification_threshold(t: Terminal) -> None:
    cfg = t.get_notification_config()
    cfg.activity_threshold = 77
    t.set_notification_config(cfg)


# name -> (apply, read); read must differ from a fresh Terminal after apply
PROBES: dict[str, Probe] = {
    # HostConfig: reverted on RIS before ARC-100.
    "set_max_transfer_size": (
        lambda t: t.set_max_transfer_size(1234),
        lambda t: t.get_max_transfer_size(),
    ),
    "set_bold_brightening": (
        lambda t: t.set_bold_brightening(False),
        lambda t: t.bold_brightening(),
    ),
    "set_conformance_level": (
        lambda t: t.set_conformance_level(2),
        lambda t: t.conformance_level(),
    ),
    "set_warning_bell_volume": (
        lambda t: t.set_warning_bell_volume(1),
        lambda t: t.warning_bell_volume(),
    ),
    "set_margin_bell_volume": (
        lambda t: t.set_margin_bell_volume(1),
        lambda t: t.margin_bell_volume(),
    ),
    "set_max_mouse_history": (
        lambda t: t.set_max_mouse_history(3),
        lambda t: t.get_max_mouse_history(),
    ),
    "set_window_position": (
        lambda t: t.set_window_position(5, 6),
        lambda t: t.window_position(),
    ),
    "set_window_iconified": (
        lambda t: t.set_window_iconified(True),
        lambda t: t.window_iconified(),
    ),
    "set_max_command_history": (lambda t: t.set_max_command_history(2), _fill_commands),
    "set_max_cwd_history": (lambda t: t.set_max_cwd_history(1), _fill_cwds),
    # Already carried by reset() (ARC-058); guarded here.
    "set_accept_osc7": (
        lambda t: t.set_accept_osc7(False),
        lambda t: t.accept_osc7(),
    ),
    "set_disable_insecure_sequences": (
        lambda t: t.set_disable_insecure_sequences(True),
        lambda t: t.disable_insecure_sequences(),
    ),
    "set_max_osc_data_length": (
        lambda t: t.set_max_osc_data_length(4096),
        lambda t: t.max_osc_data_length(),
    ),
    "set_allow_file_media": (
        lambda t: t.set_allow_file_media("all"),
        lambda t: t.get_allow_file_media(),
    ),
    "set_sixel_limits": (
        lambda t: t.set_sixel_limits(11, 22, 33),
        lambda t: t.get_sixel_limits(),
    ),
    "set_sixel_graphics_limit": (
        lambda t: t.set_sixel_graphics_limit(7),
        lambda t: t.get_sixel_graphics_limit(),
    ),
    "set_answerback_string": (
        lambda t: t.set_answerback_string("par-test"),
        lambda t: t.answerback_string(),
    ),
    "set_allow_clipboard_read": (
        lambda t: t.set_allow_clipboard_read(True),
        lambda t: t.allow_clipboard_read(),
    ),
    "set_max_clipboard_event_bytes": (
        lambda t: t.set_max_clipboard_event_bytes(99),
        lambda t: t.get_max_clipboard_event_bytes(),
    ),
    "set_max_clipboard_sync_events": (
        lambda t: t.set_max_clipboard_sync_events(9),
        lambda t: t.get_max_clipboard_sync_events(),
    ),
    "set_remote_session_id": (
        lambda t: t.set_remote_session_id("remote-1"),
        lambda t: t.remote_session_id(),
    ),
    "set_default_fg": (
        lambda t: t.set_default_fg(1, 2, 3),
        lambda t: t.default_fg(),
    ),
    "set_default_bg": (
        lambda t: t.set_default_bg(4, 5, 6),
        lambda t: t.default_bg(),
    ),
    "set_cursor_color": (
        lambda t: t.set_cursor_color(7, 8, 9),
        lambda t: t.cursor_color(),
    ),
    "set_link_color": (
        lambda t: t.set_link_color(10, 11, 12),
        lambda t: t.link_color(),
    ),
    "set_bold_color": (
        lambda t: t.set_bold_color(13, 14, 15),
        lambda t: t.bold_color(),
    ),
    "set_cursor_guide_color": (
        lambda t: t.set_cursor_guide_color(16, 17, 18),
        lambda t: t.cursor_guide_color(),
    ),
    "set_badge_color": (
        lambda t: t.set_badge_color(19, 20, 21),
        lambda t: t.badge_color(),
    ),
    "set_match_color": (
        lambda t: t.set_match_color(22, 23, 24),
        lambda t: t.match_color(),
    ),
    "set_selection_bg_color": (
        lambda t: t.set_selection_bg_color(25, 26, 27),
        lambda t: t.selection_bg_color(),
    ),
    "set_selection_fg_color": (
        lambda t: t.set_selection_fg_color(28, 29, 30),
        lambda t: t.selection_fg_color(),
    ),
    "set_use_bold_color": (
        lambda t: t.set_use_bold_color(True),
        lambda t: t.use_bold_color(),
    ),
    "set_use_underline_color": (
        lambda t: t.set_use_underline_color(True),
        lambda t: t.use_underline_color(),
    ),
    "set_faint_text_alpha": (
        lambda t: t.set_faint_text_alpha(0.25),
        lambda t: t.faint_text_alpha(),
    ),
    "set_ansi_palette_color": (
        lambda t: t.set_ansi_palette_color(1, 1, 2, 3),
        lambda t: t.get_ansi_color(1),
    ),
    "set_width_config": (
        lambda t: t.set_width_config(WidthConfig.cjk()),
        _width,
    ),
    "set_ambiguous_width": (
        lambda t: t.set_ambiguous_width(AmbiguousWidth.Wide),
        _width,
    ),
    "set_unicode_version": (
        lambda t: t.set_unicode_version(UnicodeVersion.Unicode10),
        _width,
    ),
    "set_normalization_form": (
        lambda t: t.set_normalization_form(NormalizationForm.NFD),
        lambda t: int(t.normalization_form()),
    ),
    "set_max_notifications": (
        lambda t: t.set_max_notifications(5),
        lambda t: t.get_max_notifications(),
    ),
    "set_notification_config": (_set_notification_threshold, _notification_threshold),
    "set_badge_format": (
        lambda t: t.set_badge_format("\\(session.host)"),
        lambda t: t.badge_format(),
    ),
    "set_tmux_auto_detect": (
        lambda t: t.set_tmux_auto_detect(True),
        lambda t: t.is_tmux_auto_detect(),
    ),
    "set_tmux_control_mode": (
        lambda t: t.set_tmux_control_mode(True),
        lambda t: t.is_tmux_control_mode(),
    ),
    # xterm keeps tab stops across RIS.
    "set_tab_stop": (
        lambda t: t.set_tab_stop(3),
        lambda t: t.get_tab_stops(),
    ),
}

# no comparable Python getter; the named Rust test asserts it survives
RUST_COVERED: dict[str, str] = {
    "set_max_inline_images": "ris_preserves_host_config",
    "set_max_clipboard_sync_history": "ris_preserves_host_config",
    "set_event_subscription": "ris_preserves_host_config",
    "set_trigger_enabled": "ris_preserves_host_config (registry swap)",
}

# program-visible state RIS resets by design
VT_STATE: dict[str, str] = {
    "set_title": "OSC 0/2 title",
    "set_cursor_style": "DECSCUSR",
    "set_bracketed_paste": "DECSET 2004",
    "set_focus_tracking": "DECSET 1004",
    "set_mouse_encoding": "DECSET 1005/1006/1015",
    "set_keyboard_flags": "kitty CSI = u",
    "set_modify_other_keys_mode": "XTMODKEYS",
    "set_progress": "OSC 9;4 content",
    "set_named_progress_bar": "OSC 934 content",
    "set_clipboard": "OSC 52 content",
    "set_clipboard_with_slot": "OSC 52 content",
    "set_selection": "selection over cleared screen",
    "set_badge_session_variable": "session variables (OSC 1337 SetUserVar)",
}


def test_every_setter_is_classified() -> None:
    setters = {n for n in dir(Terminal) if n.startswith("set_")}
    classified = PROBES.keys() | RUST_COVERED.keys() | VT_STATE.keys()
    assert setters - classified == set(), "classify the new setter(s)"
    assert classified - setters == set(), "stale classification entries"
    assert not (PROBES.keys() & VT_STATE.keys())
    assert not (PROBES.keys() & RUST_COVERED.keys())
    assert not (RUST_COVERED.keys() & VT_STATE.keys())


# tmux control mode routes raw bytes to the tmux line parser, so ESC c never
# reaches the VT parser; reset() is the same code path RIS runs.
RESET_VIA_API = {"set_tmux_control_mode"}


@pytest.mark.parametrize("name", sorted(PROBES))
def test_host_setting_survives_ris(name: str) -> None:
    apply, read = PROBES[name]
    baseline = read(Terminal(80, 24))
    t = Terminal(80, 24)
    apply(t)
    expected = read(t)
    assert expected != baseline, f"{name}: probe changes nothing"
    if name in RESET_VIA_API:
        t.reset()
    else:
        t.process(RIS)
    assert read(t) == expected


def test_program_changes_reset_to_configured_baseline() -> None:
    t = Terminal(80, 24)
    t.set_warning_bell_volume(2)
    t.process(b"\x1b[7 t")
    assert t.warning_bell_volume() == 7
    t.process(RIS)
    assert t.warning_bell_volume() == 2


def test_program_decscl_does_not_survive_ris() -> None:
    t = Terminal(80, 24)
    t.set_conformance_level(62)
    t.process(b'\x1b[61"p')
    assert t.conformance_level() == 1
    t.process(RIS)
    assert t.conformance_level() == 2
