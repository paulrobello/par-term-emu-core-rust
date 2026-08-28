//! Hand-written escape hatches for the `PyDictConvert` derive (ARC-003).
//!
//! The derive macro covers the uniform variant -> dict field mapping; the
//! conversions with structure it cannot express (nested theme/stats dicts,
//! constructor quirks that predate the derive) live here and are referenced
//! from `#[pydict(...)]` attributes on the protocol enums.
//!
//! Every function in this module is a verbatim move of the matching arm body
//! from the previous hand-written codec in `python_bindings/streaming.rs`,
//! including its quirks (see `tests/test_streaming_dict_api.py` for the
//! pinned behavior).

use pyo3::prelude::*;
use pyo3::types::{PyDict, PyList};

use super::protocol::{EventType, ServerMessage, ThemeInfo};

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

/// Whole-variant decode for `ServerMessage::SystemStats` (`#[pydict(to)]`).
///
/// Nested cpu/memory/disks/networks/load_average dicts; the corresponding
/// keys are omitted (not `None`) when absent or empty, as before.
#[allow(clippy::too_many_arguments)]
pub fn system_stats_to_py_dict<'py>(
    py: Python<'py>,
    cpu: &Option<super::protocol::CpuStats>,
    memory: &Option<super::protocol::MemoryStats>,
    disks: &Vec<super::protocol::DiskStats>,
    networks: &Vec<super::protocol::NetworkInterfaceStats>,
    load_average: &Option<super::protocol::LoadAverage>,
    hostname: &Option<String>,
    os_name: &Option<String>,
    os_version: &Option<String>,
    kernel_version: &Option<String>,
    uptime_secs: &Option<u64>,
    timestamp: &Option<u64>,
) -> PyResult<Bound<'py, PyDict>> {
    let dict = PyDict::new(py);
    dict.set_item("type", "system_stats")?;
    if let Some(cpu) = cpu {
        let cpu_dict = PyDict::new(py);
        cpu_dict.set_item("overall_usage_percent", cpu.overall_usage_percent)?;
        cpu_dict.set_item("physical_core_count", cpu.physical_core_count)?;
        cpu_dict.set_item("per_core_usage_percent", &cpu.per_core_usage_percent)?;
        cpu_dict.set_item("brand", &cpu.brand)?;
        cpu_dict.set_item("frequency_mhz", cpu.frequency_mhz)?;
        dict.set_item("cpu", cpu_dict)?;
    }
    if let Some(memory) = memory {
        let mem_dict = PyDict::new(py);
        mem_dict.set_item("total_bytes", memory.total_bytes)?;
        mem_dict.set_item("used_bytes", memory.used_bytes)?;
        mem_dict.set_item("available_bytes", memory.available_bytes)?;
        mem_dict.set_item("swap_total_bytes", memory.swap_total_bytes)?;
        mem_dict.set_item("swap_used_bytes", memory.swap_used_bytes)?;
        dict.set_item("memory", mem_dict)?;
    }
    if !disks.is_empty() {
        let disk_list = PyList::empty(py);
        for d in disks {
            let dd = PyDict::new(py);
            dd.set_item("name", &d.name)?;
            dd.set_item("mount_point", &d.mount_point)?;
            dd.set_item("total_bytes", d.total_bytes)?;
            dd.set_item("available_bytes", d.available_bytes)?;
            dd.set_item("kind", &d.kind)?;
            dd.set_item("file_system", &d.file_system)?;
            dd.set_item("is_removable", d.is_removable)?;
            disk_list.append(dd)?;
        }
        dict.set_item("disks", disk_list)?;
    }
    if !networks.is_empty() {
        let net_list = PyList::empty(py);
        for n in networks {
            let nd = PyDict::new(py);
            nd.set_item("name", &n.name)?;
            nd.set_item("received_bytes", n.received_bytes)?;
            nd.set_item("transmitted_bytes", n.transmitted_bytes)?;
            nd.set_item("total_received_bytes", n.total_received_bytes)?;
            nd.set_item("total_transmitted_bytes", n.total_transmitted_bytes)?;
            nd.set_item("packets_received", n.packets_received)?;
            nd.set_item("packets_transmitted", n.packets_transmitted)?;
            nd.set_item("errors_received", n.errors_received)?;
            nd.set_item("errors_transmitted", n.errors_transmitted)?;
            net_list.append(nd)?;
        }
        dict.set_item("networks", net_list)?;
    }
    if let Some(la) = load_average {
        let la_dict = PyDict::new(py);
        la_dict.set_item("one_minute", la.one_minute)?;
        la_dict.set_item("five_minutes", la.five_minutes)?;
        la_dict.set_item("fifteen_minutes", la.fifteen_minutes)?;
        dict.set_item("load_average", la_dict)?;
    }
    dict.set_item("hostname", hostname)?;
    dict.set_item("os_name", os_name)?;
    dict.set_item("os_version", os_version)?;
    dict.set_item("kernel_version", kernel_version)?;
    dict.set_item("uptime_secs", uptime_secs)?;
    dict.set_item("timestamp", timestamp)?;
    Ok(dict)
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
