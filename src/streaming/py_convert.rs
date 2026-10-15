//! Hand-written escape hatches for the `PyDictConvert` derive (ARC-003).
//!
//! The derive macro covers the uniform variant -> dict field mapping; the
//! conversions with structure it cannot express (the nested theme dict,
//! constructor quirks that predate the derive) live here and are referenced
//! from `#[pydict(...)]` attributes on the protocol enums.
//!
//! Every function in this module is a verbatim move of the matching arm body
//! from the previous hand-written codec in `python_bindings/streaming.rs`,
//! including its quirks (see `tests/test_streaming_dict_api.py` for the
//! pinned behavior).

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};

use super::protocol::{EventType, MouseEventType, ServerMessage, ThemeInfo};

/// `Connected.theme` field -> `theme` dict value.
///
/// Emits exactly `name` / `background` / `foreground` (the previous decode
/// never included `normal` / `bright`), and `None` when the theme is absent.
pub fn theme_to_py<'py>(
    py: Python<'py>,
    theme: &Option<ThemeInfo>,
) -> PyResult<Bound<'py, pyo3::types::PyAny>> {
    match theme {
        Some(t) => {
            let theme_dict = PyDict::new(py);
            theme_dict.set_item("name", &t.name)?;
            theme_dict.set_item(
                "background",
                (t.background.0, t.background.1, t.background.2),
            )?;
            theme_dict.set_item(
                "foreground",
                (t.foreground.0, t.foreground.1, t.foreground.2),
            )?;
            Ok(theme_dict.into_any())
        }
        None => Ok(py.None().into_bound(py)),
    }
}

/// `theme` kwarg -> `Connected.theme` field.
///
/// Accepts a dict with `name`/`background`/`foreground`/`normal`/`bright`;
/// returns `None` unless both palettes have exactly 8 entries (mirrors the
/// previous lenient parse: any failure yields `None`, never an error).
pub fn theme_from_py(value: Option<&Bound<'_, pyo3::types::PyAny>>) -> Option<ThemeInfo> {
    let v = value?;
    let name: String = v.get_item("name").ok()?.extract().ok()?;
    let background: (u8, u8, u8) = v.get_item("background").ok()?.extract().ok()?;
    let foreground: (u8, u8, u8) = v.get_item("foreground").ok()?.extract().ok()?;
    let normal_vec: Vec<(u8, u8, u8)> = v.get_item("normal").ok()?.extract().ok()?;
    let bright_vec: Vec<(u8, u8, u8)> = v.get_item("bright").ok()?.extract().ok()?;

    if normal_vec.len() != 8 || bright_vec.len() != 8 {
        return None;
    }

    let mut normal = [(0u8, 0u8, 0u8); 8];
    let mut bright = [(0u8, 0u8, 0u8); 8];
    for (i, c) in normal_vec.into_iter().enumerate() {
        normal[i] = c;
    }
    for (i, c) in bright_vec.into_iter().enumerate() {
        bright[i] = c;
    }

    Some(ThemeInfo {
        name,
        background,
        foreground,
        normal,
        bright,
    })
}

/// `Subscribe.events` field -> `events` list value (tag strings).
pub fn events_to_py<'py>(
    py: Python<'py>,
    events: &Vec<EventType>,
) -> PyResult<Bound<'py, pyo3::types::PyAny>> {
    let list = PyList::empty(py);
    for event in events {
        list.append(event.py_type_tag())?;
    }
    Ok(list.into_any())
}

/// `events` kwarg -> `Subscribe.events` field.
///
/// Unknown event names are silently dropped (the previous filter_map).
pub fn events_from_py(value: Option<&Bound<'_, pyo3::types::PyAny>>) -> Vec<EventType> {
    let strs: Vec<String> = value.and_then(|v| v.extract().ok()).unwrap_or_default();
    strs.iter()
        .filter_map(|s| EventType::from_py_kwargs(s, None).ok().flatten())
        .collect()
}

/// `Mouse.event_type` field -> `event_type` dict value (the wire name).
pub fn mouse_event_type_to_py<'py>(
    py: Python<'py>,
    event_type: &MouseEventType,
) -> PyResult<Bound<'py, pyo3::types::PyAny>> {
    Ok(pyo3::types::PyString::new(py, event_type.as_str()).into_any())
}

/// `event_type` kwarg -> `Mouse.event_type` field.
///
/// An absent key, a non-string value, or an unknown name is `Press` — the
/// previous `"press"` default; the server already treated every name but
/// `"release"` as a press.
pub fn mouse_event_type_from_py(value: Option<&Bound<'_, pyo3::types::PyAny>>) -> MouseEventType {
    value
        .and_then(|v| v.extract::<String>().ok())
        .and_then(|s| MouseEventType::parse(&s))
        .unwrap_or(MouseEventType::Press)
}

/// Whole-variant encode for `ServerMessage::SystemStats` (`#[pydict(from)]`).
///
/// The Python encode API ignores all kwargs and emits the empty variant
/// (system stats are server-generated), as before.
pub fn system_stats_from(_kwargs: Option<&Bound<'_, PyDict>>) -> PyResult<ServerMessage> {
    Ok(ServerMessage::system_stats(
        None,
        None,
        vec![],
        vec![],
        None,
        None,
        None,
        None,
        None,
        None,
        None,
    ))
}

/// Whole-variant encode for `ServerMessage::CwdChanged` (`#[pydict(from)]`).
///
/// Preserves the historical quirk: without a `timestamp` kwarg the plain
/// `cwd_changed(new_cwd)` constructor runs and `old_cwd`/`hostname`/
/// `username` are dropped even when passed.
pub fn cwd_changed_from(kwargs: Option<&Bound<'_, PyDict>>) -> PyResult<ServerMessage> {
    let get_str = |key: &str| -> Option<String> {
        kwargs
            .and_then(|k| k.get_item(key).ok().flatten())
            .and_then(|v| v.extract().ok())
    };
    let get_u64 = |key: &str| -> Option<u64> {
        kwargs
            .and_then(|k| k.get_item(key).ok().flatten())
            .and_then(|v| v.extract().ok())
    };

    let new_cwd = get_str("new_cwd").unwrap_or_default();
    let old_cwd = get_str("old_cwd");
    let hostname = get_str("hostname");
    let username = get_str("username");
    let timestamp = get_u64("timestamp");
    Ok(match timestamp {
        Some(ts) => ServerMessage::cwd_changed_full(old_cwd, new_cwd, hostname, username, ts),
        None => ServerMessage::cwd_changed(new_cwd),
    })
}

fn agent_entry_to_py_dict<'py>(
    py: Python<'py>,
    entry: &super::protocol::AgentEntry,
) -> PyResult<Bound<'py, PyDict>> {
    let dict = PyDict::new(py);
    dict.set_item("pane_id", entry.pane_id)?;
    dict.set_item("agent", &entry.agent)?;
    dict.set_item("state", &entry.state)?;
    dict.set_item("source", &entry.source)?;
    dict.set_item("reason", &entry.reason)?;
    Ok(dict)
}

/// Whole-variant decode for `ServerMessage::AgentRoster` (`#[pydict(to)]`).
pub fn agent_roster_to_py_dict<'py>(
    py: Python<'py>,
    agents: &Vec<super::protocol::AgentEntry>,
) -> PyResult<Bound<'py, PyDict>> {
    let dict = PyDict::new(py);
    dict.set_item("type", "agent_roster")?;
    let list = PyList::empty(py);
    for entry in agents {
        list.append(agent_entry_to_py_dict(py, entry)?)?;
    }
    dict.set_item("agents", list)?;
    Ok(dict)
}

/// Whole-variant encode for `ServerMessage::AgentRoster`: roster messages are
/// server-generated, so the Python encode API emits the empty variant.
pub fn agent_roster_from(_kwargs: Option<&Bound<'_, PyDict>>) -> PyResult<ServerMessage> {
    Ok(ServerMessage::AgentRoster { agents: vec![] })
}

/// Whole-variant decode for `ServerMessage::AgentStateChanged`.
pub fn agent_state_changed_to_py_dict<'py>(
    py: Python<'py>,
    agent: &super::protocol::AgentEntry,
    released: &bool,
) -> PyResult<Bound<'py, PyDict>> {
    let dict = PyDict::new(py);
    dict.set_item("type", "agent_state_changed")?;
    dict.set_item("agent", agent_entry_to_py_dict(py, agent)?)?;
    dict.set_item("released", *released)?;
    Ok(dict)
}

/// Whole-variant encode for `ServerMessage::AgentStateChanged`: server-generated,
/// so the Python encode API emits an empty upsert.
pub fn agent_state_changed_from(_kwargs: Option<&Bound<'_, PyDict>>) -> PyResult<ServerMessage> {
    Ok(ServerMessage::AgentStateChanged {
        agent: Default::default(),
        released: false,
    })
}

#[cfg(test)]
mod system_stats_dict {
    use super::super::protocol::*;
    use pyo3::prelude::*;

    fn populated() -> ServerMessage {
        ServerMessage::system_stats(
            Some(CpuStats {
                overall_usage_percent: 12.5,
                physical_core_count: 8,
                per_core_usage_percent: vec![1.0, 2.0],
                brand: Some("M".into()),
                frequency_mhz: Some(3200),
            }),
            Some(MemoryStats {
                total_bytes: 1,
                used_bytes: 2,
                available_bytes: 3,
                swap_total_bytes: 4,
                swap_used_bytes: 5,
            }),
            vec![DiskStats {
                name: "d".into(),
                mount_point: "/".into(),
                total_bytes: 6,
                available_bytes: 7,
                kind: "SSD".into(),
                file_system: "apfs".into(),
                is_removable: true,
            }],
            vec![NetworkInterfaceStats {
                name: "en0".into(),
                received_bytes: 1,
                transmitted_bytes: 2,
                total_received_bytes: 3,
                total_transmitted_bytes: 4,
                packets_received: 5,
                packets_transmitted: 6,
                errors_received: 7,
                errors_transmitted: 8,
            }],
            Some(LoadAverage {
                one_minute: 1.0,
                five_minutes: 2.0,
                fifteen_minutes: 3.0,
            }),
            Some("h".into()),
            Some("os".into()),
            Some("1".into()),
            Some("k".into()),
            Some(9),
            Some(10),
        )
    }

    fn partial() -> ServerMessage {
        ServerMessage::system_stats(
            None,
            Some(MemoryStats {
                total_bytes: 1,
                used_bytes: 2,
                available_bytes: 3,
                swap_total_bytes: 4,
                swap_used_bytes: 5,
            }),
            vec![],
            vec![],
            None,
            Some("h".into()),
            None,
            None,
            None,
            None,
            Some(10),
        )
    }

    /// The derived `#[pydict(nested)]` dict shape, pinned byte-for-byte to
    /// the hand-rolled converter it replaced (ARC-006): nested dicts in
    /// struct field order, and `cpu`/`memory`/`disks`/`networks`/
    /// `load_average` omitted (not `None`) when absent or empty.
    #[test]
    fn derived_dict_matches_pinned_shape() {
        let expected = [
            "{'type': 'system_stats', 'cpu': {'overall_usage_percent': 12.5, 'physical_core_count': 8, 'per_core_usage_percent': [1.0, 2.0], 'brand': 'M', 'frequency_mhz': 3200}, 'memory': {'total_bytes': 1, 'used_bytes': 2, 'available_bytes': 3, 'swap_total_bytes': 4, 'swap_used_bytes': 5}, 'disks': [{'name': 'd', 'mount_point': '/', 'total_bytes': 6, 'available_bytes': 7, 'kind': 'SSD', 'file_system': 'apfs', 'is_removable': True}], 'networks': [{'name': 'en0', 'received_bytes': 1, 'transmitted_bytes': 2, 'total_received_bytes': 3, 'total_transmitted_bytes': 4, 'packets_received': 5, 'packets_transmitted': 6, 'errors_received': 7, 'errors_transmitted': 8}], 'load_average': {'one_minute': 1.0, 'five_minutes': 2.0, 'fifteen_minutes': 3.0}, 'hostname': 'h', 'os_name': 'os', 'os_version': '1', 'kernel_version': 'k', 'uptime_secs': 9, 'timestamp': 10}",
            "{'type': 'system_stats', 'memory': {'total_bytes': 1, 'used_bytes': 2, 'available_bytes': 3, 'swap_total_bytes': 4, 'swap_used_bytes': 5}, 'hostname': 'h', 'os_name': None, 'os_version': None, 'kernel_version': None, 'uptime_secs': None, 'timestamp': 10}",
        ];
        Python::attach(|py| {
            for (msg, want) in [populated(), partial()].into_iter().zip(expected) {
                let got = msg.to_py_dict(py).unwrap().repr().unwrap().to_string();
                assert_eq!(got, want);
            }
        });
    }
}
