//! Popup command windows: the `[[client.popup]]` config shape, its
//! validation, and the floating box geometry. The attach renderer's
//! overlay lives in `attach/render/popup.rs`.
//!
//! Ported from herdr's `type = "popup"` custom commands (Apache-2.0,
//! herdr `src/popup_size.rs`, `src/config/keybinds.rs`,
//! `src/app/popup.rs`): a key opens a command in a floating terminal
//! over the tiled view without changing the layout. `width`/`height`
//! take terminal cells (an integer) or a percentage string (`"80%"`) of
//! the pane area, measure the OUTER box (border included), default to
//! half the area, and clamp to the area with a 6x4 minimum.

use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// The smallest outer popup box (herdr's minimum).
pub const MIN_WIDTH: u16 = 6;
/// The smallest outer popup box height.
pub const MIN_HEIGHT: u16 = 4;

/// One popup dimension: cells, or a percentage of the pane area.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PopupSize {
    /// Terminal cells, border included.
    Cells(u16),
    /// 1..=100 percent of the area.
    Percent(u8),
}

impl PopupSize {
    /// This size against an `available` extent.
    #[must_use]
    pub fn resolve(self, available: u16) -> u16 {
        match self {
            Self::Cells(cells) => cells,
            Self::Percent(percent) => ((u32::from(available) * u32::from(percent)) / 100) as u16,
        }
    }

    /// Parse the text spelling: `120` (cells) or `80%`.
    pub fn parse(value: &str) -> Result<Self, String> {
        if let Some(percent) = value.strip_suffix('%') {
            let percent = percent
                .parse::<u8>()
                .map_err(|_| "must be a number of cells or a percentage like 80%".to_string())?;
            if !(1..=100).contains(&percent) {
                return Err("percentage must be between 1% and 100%".to_string());
            }
            return Ok(Self::Percent(percent));
        }
        value
            .parse::<u16>()
            .map(Self::Cells)
            .map_err(|_| "must be a number of cells or a percentage like 80%".to_string())
    }

    /// The text spelling [`Self::parse`] reads back.
    #[must_use]
    pub fn spell(self) -> String {
        match self {
            Self::Cells(cells) => cells.to_string(),
            Self::Percent(percent) => format!("{percent}%"),
        }
    }
}

impl Serialize for PopupSize {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Cells(cells) => serializer.serialize_u16(*cells),
            Self::Percent(_) => serializer.serialize_str(&self.spell()),
        }
    }
}

/// One popup dimension as the file spells it, before validation: any
/// TOML value deserializes (a bad `[[client.popup]]` entry must not make
/// the whole config file unreadable — the daemon reads the same file),
/// and [`validate`] refuses what does not resolve.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SizeSpec {
    /// A valid size.
    Size(PopupSize),
    /// The raw spelling of a value that is not one (herdr's rule: an
    /// integer is cells; a string must be a percentage — `"120"` is
    /// refused, use a number for cells).
    Invalid(String),
}

impl SizeSpec {
    /// The size, `None` for an invalid spelling.
    #[must_use]
    pub fn size(&self) -> Option<PopupSize> {
        match self {
            Self::Size(size) => Some(*size),
            Self::Invalid(_) => None,
        }
    }
}

impl From<PopupSize> for SizeSpec {
    fn from(size: PopupSize) -> Self {
        Self::Size(size)
    }
}

impl Serialize for SizeSpec {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        match self {
            Self::Size(size) => size.serialize(serializer),
            Self::Invalid(raw) => serializer.serialize_str(raw),
        }
    }
}

impl<'de> Deserialize<'de> for SizeSpec {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let value = toml::Value::deserialize(deserializer)?;
        Ok(match &value {
            toml::Value::Integer(n) => u16::try_from(*n)
                .map(|cells| Self::Size(PopupSize::Cells(cells)))
                .unwrap_or_else(|_| Self::Invalid(n.to_string())),
            toml::Value::String(text) if text.ends_with('%') => PopupSize::parse(text)
                .map(Self::Size)
                .unwrap_or_else(|_| Self::Invalid(text.clone())),
            toml::Value::String(text) => Self::Invalid(text.clone()),
            other => Self::Invalid(other.to_string()),
        })
    }
}

/// One `[[client.popup]]` table as the file spells it.
#[derive(Debug, Clone, Default, PartialEq, Eq, Deserialize, Serialize)]
pub struct PopupEntry {
    /// The key matched after the prefix, tmux spelling (`t`, `C-g`).
    #[serde(default)]
    pub key: String,
    /// The shell command the popup runs (`/bin/sh` on unix).
    #[serde(default)]
    pub command: String,
    /// Outer width: cells or `"N%"`; default half the pane area.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub width: Option<SizeSpec>,
    /// Outer height: cells or `"N%"`; default half the pane area.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub height: Option<SizeSpec>,
    /// The title the box's border shows (default: the command).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub description: Option<String>,
}

/// One validated popup: the after-prefix key byte plus the entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Popup {
    /// The after-prefix key byte.
    pub key: u8,
    /// The entry it came from.
    pub entry: PopupEntry,
}

impl PopupEntry {
    /// The validated width, `None` = the default.
    #[must_use]
    pub fn width(&self) -> Option<PopupSize> {
        self.width.as_ref().and_then(SizeSpec::size)
    }

    /// The validated height, `None` = the default.
    #[must_use]
    pub fn height(&self) -> Option<PopupSize> {
        self.height.as_ref().and_then(SizeSpec::size)
    }
}

impl Popup {
    /// The border title: the description, else the command.
    #[must_use]
    pub fn title(&self) -> &str {
        self.entry
            .description
            .as_deref()
            .filter(|d| !d.trim().is_empty())
            .unwrap_or(&self.entry.command)
    }
}

/// Validate the file's popup tables: every key parses as one tmux key,
/// every command is non-empty and one line (the control wire is
/// line-delimited), and no two popups share a key.
pub fn validate(entries: &[PopupEntry]) -> Result<Vec<Popup>, String> {
    let mut popups: Vec<Popup> = Vec::with_capacity(entries.len());
    for (index, entry) in entries.iter().enumerate() {
        let key = crate::mux::config::management_key(&entry.key, &format!("popup[{index}].key"))?;
        if entry.command.trim().is_empty() {
            return Err(format!("[client] popup[{index}].command is empty"));
        }
        if entry.command.contains(['\n', '\r']) {
            return Err(format!("[client] popup[{index}].command must be one line"));
        }
        for (axis, spec) in [("width", &entry.width), ("height", &entry.height)] {
            if let Some(SizeSpec::Invalid(raw)) = spec {
                return Err(format!(
                    "[client] popup[{index}].{axis} {raw:?} is not a size (an integer of cells, or a percentage string like \"80%\")"
                ));
            }
        }
        if popups.iter().any(|p| p.key == key) {
            return Err(format!(
                "[client] popup[{index}].key {:?} is bound by an earlier popup",
                entry.key
            ));
        }
        popups.push(Popup {
            key,
            entry: entry.clone(),
        });
    }
    Ok(popups)
}

/// The after-prefix keys the fixed table and the management chords
/// already own — a popup on one of them never fires (popups match
/// last), which [`shadow_warnings`] reports.
fn bound_keys(chords: &crate::mux::config::Chords) -> Vec<u8> {
    let m = &chords.management;
    let mut keys = vec![
        chords.reload,
        chords.prefix,
        b'd',
        b'[',
        b'n',
        b'p',
        b'(',
        b')',
        b'o',
    ];
    keys.extend([
        m.split_right,
        m.split_down,
        m.kill_pane,
        m.break_pane,
        m.join_pane,
        m.palette,
        m.new_window,
        m.resize,
        m.swap_prev,
        m.swap_next,
        m.workspace_next,
        m.workspace_prev,
        m.help,
        m.picker,
        m.zoom,
        m.rename_window,
        m.rename_pane,
        m.border_cycle,
        m.label_toggle,
        m.workspace_picker,
        m.sidebar,
        m.status_bar,
        m.respawn,
        m.notify_jump,
    ]);
    keys
}

/// One warning per popup whose key a built-in chord already owns.
#[must_use]
pub fn shadow_warnings(chords: &crate::mux::config::Chords) -> Vec<String> {
    let bound = bound_keys(chords);
    chords
        .popups
        .iter()
        .filter(|p| bound.contains(&p.key))
        .map(|p| {
            format!(
                "[client] popup key {:?} is already a prefix chord — the popup never opens",
                p.entry.key
            )
        })
        .collect()
}

/// A cell rectangle: `x`, `y`, `width`, `height`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CellRect {
    /// Left column.
    pub x: u16,
    /// Top row.
    pub y: u16,
    /// Columns.
    pub width: u16,
    /// Rows.
    pub height: u16,
}

/// The popup's outer box and the terminal grid inside its border.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PopupGeometry {
    /// The box, border included.
    pub outer: CellRect,
    /// The terminal grid inside the border.
    pub inner: CellRect,
}

/// herdr's `resolve_popup_geometry`: the outer box centered over `area`,
/// half the area by default, clamped to it with a 6x4 floor; `None` when
/// the area cannot hold the minimum. The inner grid is the outer box
/// less its one-cell border (herdr also reserves a scrollbar column;
/// par-mux paints none, so the grid keeps it).
#[must_use]
pub fn resolve_geometry(
    width: Option<PopupSize>,
    height: Option<PopupSize>,
    area: CellRect,
) -> Option<PopupGeometry> {
    let default_width = (area.width / 2).max(MIN_WIDTH);
    let default_height = (area.height / 2).max(MIN_HEIGHT);
    let outer_width = width
        .map_or(default_width, |w| w.resolve(area.width))
        .max(MIN_WIDTH)
        .min(area.width);
    let outer_height = height
        .map_or(default_height, |h| h.resolve(area.height))
        .max(MIN_HEIGHT)
        .min(area.height);
    if outer_width < MIN_WIDTH || outer_height < MIN_HEIGHT {
        return None;
    }
    let outer = CellRect {
        x: area.x + (area.width - outer_width) / 2,
        y: area.y + (area.height - outer_height) / 2,
        width: outer_width,
        height: outer_height,
    };
    let inner = CellRect {
        x: outer.x + 1,
        y: outer.y + 1,
        width: outer_width - 2,
        height: outer_height - 2,
    };
    Some(PopupGeometry { outer, inner })
}

/// The settings dialog's one-line spelling of a popup: `KEY [WxH] COMMAND`
/// (`t 80%x80% lazygit`; the size part only when one is set).
#[must_use]
pub fn spell_entry(entry: &PopupEntry) -> String {
    match size_spelling(entry) {
        Some(size) => format!("{} {size} {}", entry.key, entry.command),
        None => format!("{} {}", entry.key, entry.command),
    }
}

/// `WxH` for an entry with a size (a missing axis spells `-`).
fn size_spelling(entry: &PopupEntry) -> Option<String> {
    if entry.width.is_none() && entry.height.is_none() {
        return None;
    }
    let axis = |s: Option<PopupSize>| s.map_or_else(|| "-".to_string(), PopupSize::spell);
    Some(format!("{}x{}", axis(entry.width()), axis(entry.height())))
}

/// Parse [`spell_entry`]'s form back into an entry (the description is
/// carried over from `previous`, which the one-line form cannot spell).
pub fn parse_spelling(text: &str, previous: Option<&PopupEntry>) -> Result<PopupEntry, String> {
    let text = text.trim();
    let (key, rest) = text
        .split_once(char::is_whitespace)
        .ok_or_else(|| "expected KEY [WxH] COMMAND, e.g. t 80%x80% lazygit".to_string())?;
    let rest = rest.trim_start();
    if looks_like_size(rest) {
        return Err("expected a command after the size".to_string());
    }
    let (width, height, command) = match rest.split_once(char::is_whitespace) {
        Some((first, tail)) if looks_like_size(first) => {
            let (w, h) = first.split_once('x').expect("checked by looks_like_size");
            let axis = |s: &str| -> Result<Option<PopupSize>, String> {
                if s == "-" {
                    Ok(None)
                } else {
                    PopupSize::parse(s).map(Some)
                }
            };
            (axis(w)?, axis(h)?, tail.trim())
        }
        _ => (None, None, rest),
    };
    let entry = PopupEntry {
        key: key.to_string(),
        command: command.to_string(),
        width: width.map(SizeSpec::from),
        height: height.map(SizeSpec::from),
        description: previous.and_then(|p| p.description.clone()),
    };
    validate(std::slice::from_ref(&entry))?;
    Ok(entry)
}

/// Whether `word` is a `WxH` size token (`80%x40`, `-x60%`).
fn looks_like_size(word: &str) -> bool {
    let Some((w, h)) = word.split_once('x') else {
        return false;
    };
    let axis = |s: &str| s == "-" || PopupSize::parse(s).is_ok();
    axis(w) && axis(h)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn area(width: u16, height: u16) -> CellRect {
        CellRect {
            x: 0,
            y: 0,
            width,
            height,
        }
    }

    /// herdr's `parse_cli` cases: cells, percentages, and the refusals.
    #[test]
    fn popup_size_parses_cells_and_percentages() {
        assert_eq!(PopupSize::parse("120"), Ok(PopupSize::Cells(120)));
        assert_eq!(PopupSize::parse("80%"), Ok(PopupSize::Percent(80)));
        assert_eq!(PopupSize::Percent(80).resolve(100), 80);
        assert!(PopupSize::parse("0%").is_err());
        assert!(PopupSize::parse("101%").is_err());
        assert!(PopupSize::parse("%").is_err());
        assert!(PopupSize::parse("wide").is_err());
    }

    /// The file form: an integer is cells, a string must be a
    /// percentage (herdr refuses `"120"`), and both round-trip.
    #[test]
    fn popup_tables_deserialize_and_round_trip() {
        let file: crate::mux::config::ConfigFile = toml::from_str(
            "[client]\n[[client.popup]]\nkey = \"t\"\ncommand = \"lazygit\"\nwidth = \"80%\"\nheight = 30\n\
             [[client.popup]]\nkey = \"C-g\"\ncommand = \"htop\"\ndescription = \"processes\"\n",
        )
        .expect("parses");
        let popups = file.client.popup.clone().expect("popups");
        assert_eq!(popups.len(), 2);
        assert_eq!(popups[0].width(), Some(PopupSize::Percent(80)));
        assert_eq!(popups[0].height(), Some(PopupSize::Cells(30)));
        assert_eq!(popups[1].description.as_deref(), Some("processes"));
        let text = toml::to_string(&file).expect("serializes");
        let back: crate::mux::config::ConfigFile = toml::from_str(&text).expect("re-parses");
        assert_eq!(back.client.popup, file.client.popup);
        // A bad entry never makes the FILE unreadable (the daemon reads
        // the same file); validation refuses it client-side instead.
        let bad: crate::mux::config::ConfigFile = toml::from_str(
            "[daemon]\nexit-empty = false\n[[client.popup]]\nkey = \"t\"\ncommand = \"x\"\nwidth = \"120\"\n\
             [[client.popup]]\nheight = true\n",
        )
        .expect("a bad popup entry still parses");
        assert_eq!(bad.daemon.exit_empty, Some(false), "daemon keys survive");
        let entries = bad.client.popup.expect("popups");
        let err = validate(&entries[..1]).expect_err("\"120\" refused");
        assert!(err.contains("popup[0].width \"120\""), "{err}");
        assert!(
            validate(&entries[1..]).is_err(),
            "a missing key/command is refused"
        );
        assert!(crate::mux::config::reload_client_chords(
            &crate::mux::config::ConfigFile {
                client: crate::mux::config::ClientSection {
                    popup: Some(entries),
                    ..Default::default()
                },
                ..Default::default()
            },
            &crate::mux::config::Chords::with_defaults()
        )
        .is_err());
    }

    /// Validation: keys parse, commands are one non-empty line, and keys
    /// are unique.
    #[test]
    fn validate_refuses_bad_keys_commands_and_duplicates() {
        let entry = |key: &str, command: &str| PopupEntry {
            key: key.to_string(),
            command: command.to_string(),
            ..PopupEntry::default()
        };
        let ok = validate(&[entry("t", "lazygit"), entry("C-g", "htop")]).expect("valid");
        assert_eq!(ok[0].key, b't');
        assert_eq!(ok[1].key, 0x07);
        assert!(validate(&[entry("", "x")]).is_err());
        assert!(validate(&[entry("t", "  ")]).is_err());
        assert!(validate(&[entry("t", "a\nb")]).is_err());
        assert!(validate(&[entry("t", "a"), entry("t", "b")]).is_err());
    }

    /// A popup on a built-in chord's key warns (it can never open).
    #[test]
    fn shadowed_popup_keys_warn() {
        let mut chords = crate::mux::config::Chords::with_defaults();
        chords.popups = validate(&[
            PopupEntry {
                key: "g".to_string(),
                command: "lazygit".to_string(),
                ..PopupEntry::default()
            },
            PopupEntry {
                key: "t".to_string(),
                command: "htop".to_string(),
                ..PopupEntry::default()
            },
        ])
        .expect("valid");
        let warnings = shadow_warnings(&chords);
        assert_eq!(warnings.len(), 1, "{warnings:?}");
        assert!(warnings[0].contains("\"g\""), "{warnings:?}");
    }

    /// herdr's geometry tests: percentages centered, 100% fills, a tiny
    /// cell size floors at the 6x4 minimum, and an area under the
    /// minimum refuses.
    #[test]
    fn geometry_centers_clamps_and_floors() {
        let g = resolve_geometry(
            Some(PopupSize::Percent(80)),
            Some(PopupSize::Percent(40)),
            area(100, 50),
        )
        .expect("fits");
        assert_eq!(
            g.outer,
            CellRect {
                x: 10,
                y: 15,
                width: 80,
                height: 20
            }
        );
        assert_eq!(
            g.inner,
            CellRect {
                x: 11,
                y: 16,
                width: 78,
                height: 18
            }
        );
        let full = resolve_geometry(
            Some(PopupSize::Percent(100)),
            Some(PopupSize::Percent(100)),
            area(80, 24),
        )
        .expect("fits");
        assert_eq!(full.outer, area(80, 24));
        let tiny = resolve_geometry(
            Some(PopupSize::Cells(4)),
            Some(PopupSize::Cells(2)),
            area(80, 24),
        )
        .expect("fits");
        assert_eq!((tiny.outer.width, tiny.outer.height), (6, 4));
        let default = resolve_geometry(None, None, area(80, 24)).expect("fits");
        assert_eq!((default.outer.width, default.outer.height), (40, 12));
        assert_eq!(resolve_geometry(None, None, area(5, 24)), None);
        let offset = resolve_geometry(
            None,
            None,
            CellRect {
                x: 20,
                y: 0,
                width: 60,
                height: 20,
            },
        )
        .expect("fits");
        assert_eq!((offset.outer.x, offset.outer.y), (35, 5));
    }

    /// The settings spelling round-trips, keeps the description, and
    /// treats a non-size second word as part of the command.
    #[test]
    fn settings_spelling_round_trips() {
        let entry = PopupEntry {
            key: "t".to_string(),
            command: "lazygit -p .".to_string(),
            width: Some(PopupSize::Percent(80).into()),
            height: None,
            description: Some("git".to_string()),
        };
        let spelled = spell_entry(&entry);
        assert_eq!(spelled, "t 80%x- lazygit -p .");
        assert_eq!(parse_spelling(&spelled, Some(&entry)), Ok(entry.clone()));
        let plain = parse_spelling("C-g htop -d 5", None).expect("parses");
        assert_eq!(plain.command, "htop -d 5");
        assert_eq!((plain.width, plain.height), (None, None));
        assert!(parse_spelling("t", None).is_err());
        assert!(
            parse_spelling("t 80%x40", None).is_err(),
            "a size alone has no command"
        );
    }
}
