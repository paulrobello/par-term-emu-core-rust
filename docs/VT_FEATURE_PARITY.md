# VT Feature Parity

This document provides a comprehensive reference for VT100/VT220/VT320/VT420 terminal sequence support in par-term-emu-core-rust.

## Table of Contents

- [Overview](#overview)
- [CSI Sequences](#csi-sequences)
- [ESC Sequences](#esc-sequences)
- [OSC Sequences](#osc-sequences)
- [DCS Sequences](#dcs-sequences)
- [Character Handling](#character-handling)
- [Compatibility Matrix](#compatibility-matrix)
- [Known Limitations](#known-limitations)

---

## Overview

### Compatibility Level

par-term-emu-core-rust implements extensive VT terminal compatibility:

- ✅ **VT100** - Full support
- ✅ **VT220** - Full support including editing sequences
- ✅ **VT320** - Full support
- ✅ **VT420** - Rectangle operations supported
- ✅ **xterm** - Modern extensions (256-color, true color, mouse, etc.)
- ✅ **Modern protocols** - Kitty keyboard, synchronized updates, OSC 133

### Implementation Location

The terminal implementation uses a modular structure:

**Primary directory:** `src/terminal/`

**Sequence handlers** (in `src/terminal/sequences/`):
- `csi.rs` - CSI sequence handler (`csi_dispatch_impl()`)
- `esc.rs` - ESC sequence handler (`esc_dispatch_impl()`)
- `osc.rs` - OSC sequence handler (`osc_dispatch_impl()`)
- `dcs.rs` - DCS sequence handler (`dcs_hook()` / `dcs_unhook()`)

---

## CSI Sequences

CSI (Control Sequence Introducer) sequences follow the pattern: `ESC [ params intermediates final`

### Cursor Movement

| Sequence | Name | VT Level | Notes |
|----------|------|----------|-------|
| `CSI n A` | CUU (Cursor Up) | VT100 | Param 0→1, stops at top |
| `CSI n B` | CUD (Cursor Down) | VT100 | Param 0→1, stops at bottom |
| `CSI n C` | CUF (Cursor Forward) | VT100 | Param 0→1, stops at right |
| `CSI n D` | CUB (Cursor Back) | VT100 | Param 0→1, stops at left |
| `CSI n ; m H` | CUP (Cursor Position) | VT100 | Respects origin mode, 1-indexed |
| `CSI n ; m f` | HVP (Horiz/Vert Position) | VT100 | Identical to CUP |
| `CSI n E` | CNL (Cursor Next Line) | VT100 | Param 0→1, move to start of line |
| `CSI n F` | CPL (Cursor Previous Line) | VT100 | Param 0→1, move to start of line |
| `CSI n G` | CHA (Cursor Horiz Absolute) | VT100 | Param 0→1, 1-indexed column |
| `CSI n d` | VPA (Vertical Position Absolute) | VT100 | Param 0→1, 1-indexed row |

**Key Implementation Details:**
- All cursor movement respects terminal boundaries
- Parameter 0 is treated as 1 per VT specification
- Origin mode (DECOM) affects CUP/HVP addressing
- Left/right margins (DECLRMM) constrain horizontal movement

### Erasing and Clearing

| Sequence | Name | VT Level | Notes |
|----------|------|----------|-------|
| `CSI n J` | ED (Erase in Display) | VT100 | 0=below, 1=above, 2=all, 3=all+scrollback |
| `CSI n K` | EL (Erase in Line) | VT100 | 0=right, 1=left, 2=entire line |
| `CSI n X` | ECH (Erase Characters) | VT220 | Param 0→1, erase n chars from cursor |

**Erase Behavior:**
- Erased cells use current background color
- Attributes are reset to defaults
- Cursor position unchanged (except ED which may clear graphics)

### Scrolling

| Sequence | Name | VT Level | Notes |
|----------|------|----------|-------|
| `CSI n S` | SU (Scroll Up) | VT100 | Param 0→1, scroll region n lines up |
| `CSI n T` | SD (Scroll Down) | VT100 | Param 0→1, scroll region n lines down |
| `CSI t ; b r` | DECSTBM (Set Scroll Region) | VT100 | Set top/bottom margins (1-indexed) |

**Scroll Region Behavior:**
- Default region is entire screen (rows 1 to n)
- Affects IL, DL, IND, RI, LF behavior
- Origin mode makes cursor relative to region

### Line and Character Editing (VT220)

| Sequence | Name | VT Level | Notes |
|----------|------|----------|-------|
| `CSI n L` | IL (Insert Lines) | VT220 | Param 0→1, respects scroll region |
| `CSI n M` | DL (Delete Lines) | VT220 | Param 0→1, respects scroll region |
| `CSI n @` | ICH (Insert Characters) | VT220 | Param 0→1, shifts line right |
| `CSI n P` | DCH (Delete Characters) | VT220 | Param 0→1, shifts line left |

**Line Editing Behavior:**
- IL/DL only affect rows within scroll region
- New/revealed lines are blank with default attributes
- Respects left/right margins when DECLRMM is enabled

### Rectangle Operations (VT420)

| Sequence | Name | VT Level | Notes |
|----------|------|----------|-------|
| `CSI Pc;Pt;Pl;Pb;Pr $ x` | DECFRA (Fill Rectangle) | VT420 | Fill area with character Pc |
| `CSI Pts;Pls;Pbs;Prs;Pps;Ptd;Pld $ v` | DECCRA (Copy Rectangle) | VT420 | Copy rectangular region |
| `CSI Pt;Pl;Pb;Pr $ z` | DECERA (Erase Rectangle) | VT420 | Erase rectangular area |
| `CSI Pt;Pl;Pb;Pr $ {` | DECSERA (Selective Erase) | VT420 | Selective erase rectangle |
| `CSI Pt;Pl;Pb;Pr;Ps $ r` | DECCARA (Change Attributes) | VT420 | Change attributes in rectangle |
| `CSI Pt;Pl;Pb;Pr;Ps $ t` | DECRARA (Reverse Attributes) | VT420 | Reverse attributes in rectangle |
| `CSI Pi;Pg;Pt;Pl;Pb;Pr * y` | DECRQCRA (Request Checksum) | VT420 | Request rectangle checksum |

**Rectangle Operation Notes:**
- All coordinates are 1-indexed
- DECFRA fills with a single character (default space)
- DECCRA supports page parameter but uses current screen
- DECERA erases rectangular area unconditionally (ignores character protection)
- DECSERA selectively erases, preserving protected/guarded characters (set via DECSCA)
- DECCARA applies SGR attributes: 0 (reset), 1 (bold), 4 (underline), 5 (blink), 7 (reverse), 8 (hidden)
- DECRARA reverses attributes: 0 (all), 1 (bold), 4 (underline), 5 (blink), 7 (reverse), 8 (hidden)
- DECRQCRA returns DCS Pi ! ~ xxxx ST with 16-bit checksum

### Tab Control

| Sequence | Name | VT Level | Notes |
|----------|------|----------|-------|
| `CSI n I` | CHT (Cursor Forward Tab) | VT100 | Param 0→1, advance n tab stops |
| `CSI n Z` | CBT (Cursor Backward Tab) | VT100 | Param 0→1, back n tab stops |
| `CSI n g` | TBC (Tab Clear) | VT100 | 0=current, 3=all tabs |

**Tab Stop Behavior:**
- Default tab stops every 8 columns
- HTS (ESC H) sets tab at current column
- Tabs respect left/right margins

### SGR (Select Graphic Rendition)

`CSI n [; n ...] m` - Set character attributes

**Implementation:** `csi_dispatch_impl()` in `src/terminal/sequences/csi.rs`

#### Basic Attributes

| Code | Attribute | VT Level | Notes |
|------|-----------|----------|-------|
| 0 | Reset all | VT100 | Clear all attributes, default colors |
| 1 | Bold | VT100 | Bright/bold text |
| 2 | Dim | VT100 | Faint/dim text |
| 3 | Italic | VT100 | Italic text |
| 5 | Blink | VT100 | Blinking text |
| 7 | Reverse | VT100 | Swap foreground/background |
| 8 | Hidden | VT100 | Invisible text |
| 9 | Strikethrough | VT100 | Crossed-out text |

#### Underline Styles

| Code | Style | VT Level | Notes |
|------|-------|----------|-------|
| 4 | Single underline | VT100 | Standard underline |
| 4:0 | No underline | xterm | Sub-parameter syntax |
| 4:1 | Single underline | xterm | Straight line |
| 4:2 | Double underline | xterm | Two lines |
| 4:3 | Curly underline | xterm | Wavy/curly |
| 4:4 | Dotted underline | xterm | Dotted line |
| 4:5 | Dashed underline | xterm | Dashed line |
| 21 | Double underline | VT100 | Alternative syntax |

#### Reset Attributes

| Code | Reset | VT Level |
|------|-------|----------|
| 22 | Not bold/dim | VT100 |
| 23 | Not italic | VT100 |
| 24 | Not underlined | VT100 |
| 25 | Not blinking | VT100 |
| 27 | Not reversed | VT100 |
| 28 | Not hidden | VT100 |
| 29 | Not strikethrough | VT100 |

#### Standard Colors (3-bit / 4-bit)

| Code | Color | Type |
|------|-------|------|
| 30-37 | Black, Red, Green, Yellow, Blue, Magenta, Cyan, White | Foreground |
| 40-47 | Black, Red, Green, Yellow, Blue, Magenta, Cyan, White | Background |
| 90-97 | Bright Black...Bright White | Foreground (aixterm) |
| 100-107 | Bright Black...Bright White | Background (aixterm) |

#### Extended Colors

**256-Color Mode:**
```
CSI 38 ; 5 ; n m    - Set foreground to color n (0-255)
CSI 48 ; 5 ; n m    - Set background to color n (0-255)
```

**24-bit True Color:**
```
CSI 38 ; 2 ; r ; g ; b m    - Set foreground RGB
CSI 48 ; 2 ; r ; g ; b m    - Set background RGB
```

**Default Colors:**
```
CSI 39 m    - Default foreground
CSI 49 m    - Default background
```

**Implementation:** Extended color parsing in `csi_dispatch()`

**Color Parsing Notes:**
- Supports both colon (`:`) and semicolon (`;`) separators
- 256-color palette: 0-15 (standard), 16-231 (6×6×6 cube), 232-255 (grayscale)
- True color uses 8-bit RGB values (0-255)

### Mode Setting

#### Standard Modes (SM/RM)

`CSI n h` - Set Mode
`CSI n l` - Reset Mode

| Mode | Name | Default |
|------|------|---------|
| 4 | IRM (Insert/Replace Mode) | Replace |
| 20 | LNM (Line Feed/New Line Mode) | LF only |

#### DEC Private Modes (DECSET/DECRST)

`CSI ? n h` - Set Private Mode
`CSI ? n l` - Reset Private Mode

**Implementation:** `csi_dispatch_impl()` in `src/terminal/sequences/csi.rs`

##### Cursor and Display Modes

| Mode | Name | Default | Description |
|------|------|---------|-------------|
| 1 | DECCKM | Normal | Application cursor keys |
| 6 | DECOM | Absolute | Origin mode (scroll region relative) |
| 7 | DECAWM | Enabled | Auto wrap mode |
| 25 | DECTCEM | Visible | Text cursor enable |
| 69 | DECLRMM | Disabled | Left/right margin mode |

##### Screen Buffer Modes

| Mode | Name | Default | Description |
|------|------|---------|-------------|
| 47 | Alt Screen | Primary | Use alternate screen buffer |
| 1047 | Alt Screen | Primary | Use alternate screen (xterm) |
| 1048 | Save Cursor | - | Save/restore cursor position |
| 1049 | Save + Alt | Primary | Save cursor + alternate screen |

**Alternate Screen Notes:**
- No scrollback buffer in alternate screen
- Commonly used by full-screen applications (vim, less, etc.)
- Mode 1049 combines cursor save with screen switch

##### Mouse Tracking Modes

| Mode | Name | Default | Description |
|------|------|---------|-------------|
| 9 | X10 Mouse | Off | X10 compatibility (deprecated) |
| 1000 | VT200 Mouse | Off | Normal tracking (press + release) |
| 1002 | Button Event | Off | Press + release + drag |
| 1003 | Any Event | Off | All mouse motion |
| 1004 | Focus | Off | Focus in/out events |

##### Mouse Encoding Modes

| Mode | Name | Default | Description |
|------|------|---------|-------------|
| 1005 | UTF-8 Mouse | Off | UTF-8 extended coordinates |
| 1006 | SGR Mouse | Off | SGR encoding (recommended) |
| 1015 | URXVT Mouse | Off | URXVT extended coordinates |

**Mouse Encoding Notes:**
- Default encoding limited to 223 columns/rows
- SGR (`CSI < ... M/m`) is recommended for modern applications
- SGR supports button release distinction

##### Modern Extensions

| Mode | Name | Default | Description |
|------|------|---------|-------------|
| 2004 | Bracketed Paste | Off | Wrap pasted text in escape sequences |
| 2026 | Synchronized Update | Off | Batch updates for flicker-free rendering |

### Attribute Change Extent (VT420)

`CSI Ps * x` - DECSACE (Select Attribute Change Extent)

**Parameters:**
- `Ps = 0` or `1`: Stream mode (wraps at line boundaries)
- `Ps = 2`: Rectangle mode (exact rectangular boundaries, default)

**Notes:**
- Affects how DECCARA and DECRARA apply attributes
- Stream mode follows text flow and wraps at margins
- Rectangle mode strictly respects rectangular boundaries
- Default is rectangle mode (2)

**Implementation:** DECSACE handler in `src/terminal/sequences/csi.rs`

### Character Protection (VT420)

`CSI ? Ps " q` - DECSCA (Select Character Protection Attribute)

**Parameters:**
- `Ps = 0` or `2`: Characters are NOT protected (default) - DECSED and DECSERA can erase
- `Ps = 1`: Characters ARE protected - DECSED and DECSERA cannot erase

**Notes:**
- When protection is enabled (Ps=1), subsequently printed characters are marked as "guarded"
- DECSERA (Selective Erase Rectangular Area) respects the guarded flag and skips protected cells
- DECERA (Erase Rectangular Area) does NOT respect protection and erases all cells
- The guarded flag is stored per-cell in the `CellFlags.guarded` field
- Commonly used for protecting status lines or menu headers from accidental erasure

**Implementation:**
- DECSCA handler in `src/terminal/sequences/csi.rs`
- Character printing applies guarded flag in `src/terminal/write.rs`
- Grid selective erase method `erase_rectangle()` in `src/grid.rs`
- Grid unconditional erase method `erase_rectangle_unconditional()` in `src/grid.rs`

**Sequence Examples:**
```
CSI ? 1 " q        Enable protection
Hello World        (these chars are protected)
CSI ? 0 " q        Disable protection
Normal text        (these chars are NOT protected)
CSI 1 ; 1 ; 5 ; 20 $ {    DECSERA - only erases unprotected text
```

### Cursor Style

| Sequence | Style | VT Level |
|----------|-------|----------|
| `CSI 0 SP q` | Blinking block | xterm |
| `CSI 1 SP q` | Blinking block | xterm |
| `CSI 2 SP q` | Steady block | xterm |
| `CSI 3 SP q` | Blinking underline | xterm |
| `CSI 4 SP q` | Steady underline | xterm |
| `CSI 5 SP q` | Blinking bar | xterm |
| `CSI 6 SP q` | Steady bar | xterm |

**Note:** Cursor rendering is handled by the host application.

### Device Queries

#### Primary Device Attributes (DA)

`CSI c` or `CSI 0 c` - Request terminal identity

**Response:** `CSI ? 62 ; 1 ; 4 ; 6 ; 9 ; 15 ; 22 c`

**Meaning:**
- `62` - VT220
- `1` - 132 columns
- `4` - Sixel graphics
- `6` - Selective erase
- `9` - National replacement character sets
- `15` - Technical character set
- `22` - Color text

**Implementation:** `csi_dispatch_impl()` in `src/terminal/sequences/csi.rs`

#### Secondary Device Attributes

`CSI > c` - Request terminal version

**Response:** `CSI > 82 ; 10000 ; 0 c`
- `82` - Terminal type (arbitrary)
- `10000` - Version
- `0` - ROM cartridge

**Implementation:** `csi_dispatch_impl()` in `src/terminal/sequences/csi.rs`

#### Device Status Report (DSR)

| Sequence | Query | Response |
|----------|-------|----------|
| `CSI 5 n` | Status | `CSI 0 n` (OK) |
| `CSI 6 n` | Cursor Position | `CSI row ; col R` |

**CPR (Cursor Position Report) Notes:**
- Row and column are 1-indexed
- Respects origin mode (reports relative to scroll region if DECOM is set)

#### Mode Query (DECRQM)

`CSI ? mode $ p` - Query DEC private mode state

**Response:** `CSI ? mode ; state $ y`

**States:**
- `0` - Not recognized
- `1` - Set
- `2` - Reset
- `3` - Permanently set
- `4` - Permanently reset

**Implementation:** `csi_dispatch_impl()` in `src/terminal/sequences/csi.rs`

**Supported Modes:** 1, 6, 7, 25, 47, 69, 1000, 1002, 1003, 1004, 1005, 1006, 1015, 1047, 1048, 1049, 2004, 2026

#### Terminal Parameters (DECREQTPARM)

`CSI x` or `CSI 0 x` or `CSI 1 x` - Request terminal parameters

**Response:** `CSI sol ; 1 ; 1 ; 120 ; 120 ; 1 ; 0 x`
- `sol` - Solicited (2) or unsolicited (3)
- Parity, bits, transmission speed, receive speed, clock, flags

**Implementation:** `csi_dispatch_impl()` in `src/terminal/sequences/csi.rs`

### Window Operations (XTWINOPS)

`CSI Ps ; ... t`

| Ps | Operation | Response | Notes |
|----|-----------|----------|-------|
| 14 | Report pixel size | `CSI 4 ; height ; width t` | Reports terminal pixel dimensions |
| 18 | Report text size | `CSI 8 ; rows ; cols t` | Reports character grid size |
| 22 | Push title | None | Push title to stack |
| 23 | Pop title | None | Pop title from stack |
| Other | Ignored | None | Logged but not implemented |

**Notes:**
- Title stack maintains separate stacks for icon and window titles
- Most window manipulation commands are not implemented for security
- Pixel dimensions default to 0 if not configured

### Kitty Keyboard Protocol

#### Set Keyboard Flags

`CSI = flags ; mode u`

**Modes:**
- `0` or omitted - Disable all flags
- `1` - Set flags (bitwise OR)
- `2` - Lock flags (cannot be changed)
- `3` - Report current flags

**Implementation:** `csi_dispatch_impl()` in `src/terminal/sequences/csi.rs`

#### Query Flags

`CSI ? u` - Query current keyboard flags

**Response:** `CSI ? flags u`

**Implementation:** `csi_dispatch_impl()` in `src/terminal/sequences/csi.rs`

#### Push/Pop Flags

| Sequence | Operation |
|----------|-----------|
| `CSI > flags u` | Push flags to stack |
| `CSI < count u` | Pop flags (count times) |

**Notes:**
- Separate stacks for primary and alternate screens
- Default stack depth: 256 levels
- Flags control event reporting and key disambiguation

**Implementation:** `csi_dispatch_impl()` in `src/terminal/sequences/csi.rs`

### Left/Right Margins

`CSI Pl ; Pr s` - DECSLRM (Set Left/Right Margins)

**Notes:**
- Only works when DECLRMM (mode ?69) is enabled
- Otherwise, `CSI s` saves cursor position (ANSI.SYS)
- Margins are 1-indexed
- Affects cursor movement, scrolling, and editing

**Implementation:** `csi_dispatch_impl()` in `src/terminal/sequences/csi.rs`

### Cursor Save/Restore (ANSI.SYS)

| Sequence | Operation | Notes |
|----------|-----------|-------|
| `CSI s` | Save Cursor | Only if DECLRMM disabled |
| `CSI u` | Restore Cursor | Only if no intermediates |

**Note:** These are legacy ANSI.SYS sequences. Prefer `ESC 7` / `ESC 8` (DECSC/DECRC).

---

## ESC Sequences

ESC (Escape) sequences follow the pattern: `ESC final`

**Implementation:** `esc_dispatch_impl()` in `src/terminal/sequences/esc.rs`

| Sequence | Name | VT Level | Description |
|----------|------|----------|-------------|
| `ESC 7` | DECSC | VT100 | Save cursor (position, colors, attributes) |
| `ESC 8` | DECRC | VT100 | Restore cursor state |
| `ESC H` | HTS | VT100 | Set tab stop at current column |
| `ESC M` | RI | VT100 | Reverse index (move up, scroll down at top) |
| `ESC D` | IND | VT100 | Index (move down, scroll up at bottom) |
| `ESC E` | NEL | VT100 | Next line (CR + LF with scroll) |
| `ESC c` | RIS | VT100 | Reset to initial state (full terminal reset) |

### Cursor Save/Restore Details

**DECSC (ESC 7) saves:**
- Cursor position (column, row)
- Graphic rendition (SGR attributes)
- Character set (G0/G1)
- Origin mode state (DECOM)
- Wrap flag state

**DECRC (ESC 8) restores:**
- All saved cursor state
- If no save state exists, moves cursor to home position

### Reverse Index (RI) Behavior

- Moves cursor up one line
- If at top of scroll region, scrolls region down
- Respects scroll region boundaries
- New line filled with blanks using current background color

### Index (IND) Behavior

- Moves cursor down one line
- If at bottom of scroll region, scrolls region up
- Respects scroll region boundaries
- Similar to LF but always moves down (ignoring LNM mode)

### Reset (RIS) Behavior

**Full terminal reset includes:**
- Clear primary and alternate screens
- Reset all modes to defaults (DECAWM, DECOM, etc.)
- Clear scroll regions
- Reset tabs to default (every 8 columns)
- Clear title
- Reset character attributes (SGR)
- Move cursor to home (0, 0)
- Clear saved cursor state
- Reset mouse tracking and encoding
- Clear keyboard protocol flags

---

## OSC Sequences

OSC (Operating System Command) sequences follow: `ESC ] Ps ; Pt ST`
where `ST` is either `ESC \` or `BEL` (`\x07`)

**Implementation:** `osc_dispatch_impl()` in `src/terminal/sequences/osc.rs`

### Title and Icon

| Sequence | Operation | Notes |
|----------|-----------|-------|
| `OSC 0 ; title ST` | Set icon + window title | Sets both simultaneously |
| `OSC 2 ; title ST` | Set window title | Window title only |

### Directory Tracking

| Sequence | Operation | Notes |
|----------|-----------|-------|
| `OSC 7 ; file://host/cwd ST` | Set working directory | Can be disabled via `accept_osc7` |

**Format:** `file://hostname/path` (URL-encoded path)

### Hyperlinks (OSC 8)

`OSC 8 ; params ; URI ST`

**Implementation:** `osc_dispatch_impl()` in `src/terminal/sequences/osc.rs`

**Features:**
- Full URI support (http, https, file, etc.)
- Optional `id=...` parameter for link deduplication
- Links stored separately from text
- Can be disabled via `disable_insecure_sequences`

**Example:**
```
OSC 8 ; ; https://example.com ST clickable text OSC 8 ; ; ST
OSC 8 ; id=unique123 ; https://example.com ST same link OSC 8 ; ; ST
```

### Notifications

#### iTerm2 Notifications

`OSC 9 ; message ST`

**Implementation:** `osc_dispatch_impl()` in `src/terminal/sequences/osc.rs`
**Security:** Can be blocked via `disable_insecure_sequences`

#### urxvt Notifications

`OSC 777 ; notify ; title ; body ST`

**Implementation:** `osc_dispatch_impl()` in `src/terminal/sequences/osc.rs`
**Security:** Can be blocked via `disable_insecure_sequences`

### Clipboard (OSC 52)

`OSC 52 ; selection ; data ST`

**Implementation:** `osc_dispatch_impl()` in `src/terminal/sequences/osc.rs`

**Selection targets:**
- `c` - Clipboard
- `p` - Primary selection
- `s` - Secondary selection

**Operations:**
- Write: `OSC 52 ; c ; base64_data ST`
- Query: `OSC 52 ; c ; ? ST` (requires `allow_clipboard_read`)

**Security:**
- Write always permitted
- Read requires `allow_clipboard_read = true`
- Can be fully blocked via `disable_insecure_sequences`

**Response (when querying):** `OSC 52 ; c ; base64_data ST`

### Color Queries

| Sequence | Query | Response |
|----------|-------|----------|
| `OSC 10 ; ? ST` | Default foreground | `OSC 10 ; rgb:rrrr/gggg/bbbb ST` |
| `OSC 11 ; ? ST` | Default background | `OSC 11 ; rgb:rrrr/gggg/bbbb ST` |
| `OSC 12 ; ? ST` | Cursor color | `OSC 12 ; rgb:rrrr/gggg/bbbb ST` |

**Format:** 16-bit RGB values (0000-ffff) per component

**Example response:** `OSC 10 ; rgb:ffff/ffff/ffff ST` (white)

### Shell Integration (OSC 133)

`OSC 133 ; marker ; ... ST`

**Implementation:** `osc_dispatch_impl()` in `src/terminal/sequences/osc.rs`

**Markers:**
- `A` - Prompt start
- `B` - Prompt end / command start
- `C` - Command end / output start
- `D ; exit_code` - Output end with exit code

**Usage:** Enables semantic markup of shell prompt, command, and output zones for:
- Smart scrolling (jump between commands)
- Command extraction
- Exit code tracking
- Output selection

**Example sequence:**
```
OSC 133 ; A ST           # Prompt starts
OSC 133 ; B ST           # Command starts
(user types command)
OSC 133 ; C ST           # Output starts
(command output)
OSC 133 ; D ; 0 ST       # Command finished with exit code 0
```

---

## DCS Sequences

DCS (Device Control String) sequences follow: `ESC P ... ESC \`

### Sixel Graphics

`DCS Pa ; Pb ; Ph q ... ST`

**Implementation:** `dcs_hook()` / `dcs_unhook()` in `src/terminal/sequences/dcs.rs`

**Raster Attributes:**
- `Pa` - Pixel aspect ratio
- `Pb` - Background mode (1=leave current, 2=use background color)
- `Ph` - Horizontal grid size

**Sixel Commands:**

| Command | Syntax | Operation |
|---------|--------|-----------|
| Color select | `#Pc` | Select color Pc |
| Color define | `#Pc ; Pu ; Px ; Py ; Pz` | Define color RGB or HSL |
| Raster attributes | `" Pa ; Pb ; Ph ; Pv` | Set image dimensions |
| Repeat | `! Pn s` | Repeat sixel s count Pn times |
| Graphics CR | `$` | Carriage return (column 0) |
| Graphics LF | `-` | Line feed (next sixel row) |
| Sixel data | `? - ~` | Draw sixels (6 pixels vertical) |

**Color Definition Modes:**
- `Pu = 1` - HSL color space
- `Pu = 2` - RGB color space

**Features:**
- Up to 256 colors (palette indices 0-255)
- Repeat operator for compression
- Raster attributes for size declaration
- Half-block rendering fallback for terminals without Sixel support

**Security:** Can be blocked via `disable_insecure_sequences`

**Implementation:** Sixel rendering in `src/sixel.rs` and DCS handlers in `src/terminal/sequences/dcs.rs`

---

## Character Handling

**Implementation:** VTE parser callbacks in `src/terminal/mod.rs` and character writing in `src/terminal/write.rs`

### Basic Characters

| Character | Hex | Name | Behavior |
|-----------|-----|------|----------|
| BS | 0x08 | Backspace | Move cursor left (stop at left margin) |
| HT | 0x09 | Tab | Move to next tab stop |
| LF | 0x0A | Line Feed | Move down (scroll at bottom), CR if LNM |
| CR | 0x0D | Carriage Return | Move to start of line (respects left margin) |
| Printable | 0x20-0x7E, 0x80+ | Text | Display character |

### Wide Character Support

**Implementation:** Character width detection in `src/terminal/write.rs`

**Features:**
- Detects wide characters (East Asian Width property)
- Allocates 2 columns for wide characters (CJK, emoji)
- Uses spacer cells for wide character continuations
- Proper handling of wide characters at line boundaries

**Width Detection:**
- Uses Unicode `EastAsianWidth` property
- Wide (W) and Fullwidth (F) characters occupy 2 cells
- Combining marks treated as width 0 (future enhancement)

### Auto-Wrap Mode (DECAWM)

**Implementation:** Character printing logic in `src/terminal/write.rs`

**Behavior:**
- When enabled (default): Characters at right margin wrap to next line
- When disabled: Characters at right margin overwrite last column
- Delayed wrap: Wrap occurs when next character is written
- Wrap flag persists across cursor movements

### Insert Mode (IRM)

**Implementation:** Character insertion logic in `src/terminal/write.rs`

**Behavior:**
- When enabled: New characters shift existing characters right
- When disabled (default): New characters replace existing characters
- Shifted characters that exceed right margin are lost

### Tab Stops

**Implementation:** Tab handling in character printing (`src/terminal/write.rs`), HTS in `esc_dispatch_impl()` (`src/terminal/sequences/esc.rs`), TBC in `csi_dispatch_impl()` (`src/terminal/sequences/csi.rs`)

**Behavior:**
- Default tab stops every 8 columns (columns 8, 16, 24, ...)
- HTS (`ESC H`) sets tab at current column
- TBC (`CSI g`) clears tab stops
  - `CSI 0 g` - Clear tab at current column
  - `CSI 3 g` - Clear all tab stops
- CHT (`CSI I`) advances n tab stops forward
- CBT (`CSI Z`) moves n tab stops backward

---

## Compatibility Matrix

### VT100 Compatibility

| Feature Category | Support | Notes |
|------------------|---------|-------|
| Cursor movement | ✅ Full | CUU, CUD, CUF, CUB, CUP, HVP |
| Erasing | ✅ Full | ED, EL |
| Scrolling | ✅ Full | IND, RI, NEL, DECSTBM |
| Tabs | ✅ Full | HT, HTS, TBC |
| SGR basic | ✅ Full | Bold, reverse, underline, etc. |
| Character sets | ❌ Not implemented | G0/G1 switching (not needed for UTF-8) |
| Keypad modes | ⚠️ Partial | Mode switching only (key translation in host) |

### VT220 Compatibility

| Feature Category | Support | Notes |
|------------------|---------|-------|
| Line editing | ✅ Full | IL, DL, ICH, DCH, ECH |
| 8-bit controls | ✅ Full | Via UTF-8 encoding |
| Soft fonts | ❌ Not implemented | DECDLD (rarely used) |
| DRCS | ❌ Not implemented | Downloadable character sets |

### VT420 Compatibility

| Feature Category | Support | Notes |
|------------------|---------|-------|
| Rectangle operations | ✅ Full | DECFRA, DECCRA, DECERA, DECSERA, DECCARA, DECRARA |
| Rectangle checksum | ✅ Full | DECRQCRA (request checksum) |
| Attribute change extent | ✅ Full | DECSACE (stream/rectangle mode) |
| Left/Right margins | ✅ Full | DECLRMM, DECSLRM |
| Character protection | ✅ Full | DECSCA with selective erase (DECSERA) |

### xterm Compatibility

| Feature Category | Support | Notes |
|------------------|---------|-------|
| 256-color | ✅ Full | SGR 38;5;n and 48;5;n |
| True color (24-bit) | ✅ Full | SGR 38;2;r;g;b and 48;2;r;g;b |
| Mouse tracking | ✅ Full | X10, Normal, Button, Any modes |
| Mouse encoding | ✅ Full | Default, UTF-8, SGR, URXVT |
| Focus tracking | ✅ Full | Mode 1004 |
| Bracketed paste | ✅ Full | Mode 2004 |
| Alternate screen | ✅ Full | Modes 47, 1047, 1049 |
| Window ops | ⚠️ Partial | Size reporting and title stack only |
| Sixel graphics | ✅ Full | Full DCS Sixel with half-block fallback |

### Modern Protocol Support

| Protocol | Support | Implementation | Notes |
|----------|---------|----------------|-------|
| Kitty Keyboard | ✅ Full | `src/terminal/sequences/csi.rs` | Flags, push/pop, query |
| Synchronized Updates | ✅ Full | Mode 2026 | Flicker-free rendering |
| OSC 8 Hyperlinks | ✅ Full | `src/terminal/sequences/osc.rs` | With deduplication |
| OSC 52 Clipboard | ✅ Full | `src/terminal/sequences/osc.rs` | Read/write with security controls |
| OSC 133 Shell Integration | ✅ Full | `src/terminal/sequences/osc.rs` | Prompt/command/output markers |
| OSC 7 Directory Tracking | ✅ Full | `src/terminal/sequences/osc.rs` | URL-encoded paths |
| Underline styles | ✅ Full | `src/terminal/sequences/csi.rs` | 6 different styles |

---

## Known Limitations

### Not Implemented

1. **Character Set Switching (G0/G1)**
   - VT100/VT220 character set selection
   - DEC Special Graphics
   - **Reason:** UTF-8 support makes this obsolete
   - **Impact:** Minimal (old applications only)

2. **Soft Fonts (DECDLD)**
   - Downloadable character sets
   - **Reason:** Complex, rarely used
   - **Impact:** Very low (almost never used)

3. **Protected Areas (DECSCA)**
   - Character protection attribute
   - DECSERA selective erase respects protection
   - **Status:** DECSERA implemented, but protection attribute not set
   - **Impact:** Low (rarely used feature)

4. **Most XTWINOPS Operations**
   - Window resize, minimize, raise, etc.
   - **Reason:** Security concerns
   - **Implemented:** Size reporting (14, 18) and title stack (22, 23) only
   - **Impact:** Low (most are security risks anyway)

5. **CSI q without SP**
   - Different from DECSCUSR (`CSI SP q`)
   - **Impact:** Unknown (undocumented sequence)

### Implementation Notes

#### Parameter 0 Handling

All cursor movement and editing sequences correctly treat parameter 0 as 1 per VT specification:
- Cursor movement: CUU, CUD, CUF, CUB, CNL, CPL, CHA, VPA
- Editing: IL, DL, ICH, DCH, ECH
- Scrolling: SU, SD
- Tabs: CHT, CBT

**VT Spec Compliance:** When a sequence expects a count parameter and receives 0 or no parameter, it defaults to 1.

#### Origin Mode

Origin mode (DECOM) affects:
- **CUP/HVP:** Cursor positioning relative to scroll region
- **Cursor queries (DSR 6):** Position reported relative to scroll region
- **Home position:** (0,0) in absolute mode, (scroll_region_top, 0) in origin mode

#### Scroll Regions

**Top/Bottom (DECSTBM):**
- Affects: IND, RI, LF (at boundaries), IL, DL, SU, SD
- Default: Entire screen (rows 0 to rows-1)

**Left/Right (DECSLRM):**
- Requires DECLRMM mode enabled
- Affects: Cursor wrapping, line editing (ICH, DCH)
- Default: Entire width (columns 0 to cols-1)

#### Alternate Screen

- **No scrollback:** Scrollback buffer disabled in alternate screen
- **Separate cursor:** Cursor position independent from primary screen
- **Clear on switch:** Alternate screen cleared when activated
- **Mode variants:**
  - Mode 47: Basic alternate screen
  - Mode 1047: xterm alternate screen (identical behavior)
  - Mode 1049: Alternate screen + cursor save/restore

---

## Testing and Validation

### VT Test Suites

The implementation is tested with:
- Manual VT sequence testing
- Python integration tests (`tests/test_terminal.py`)
- TUI application testing (Textual integration)

### Recommended Test Applications

To validate VT compatibility, test with:
- `vttest` - Comprehensive VT100/VT220/VT420 test suite
- `vim` - Cursor movement, alternate screen, mouse
- `less` - Alternate screen, scrolling
- `tmux` - Complex scrolling regions, alternate screen
- `htop` - Mouse tracking, color, updates
- `emacs -nw` - Full terminal capabilities

### Known Working Applications

- Vim/Neovim
- Emacs
- Less/More
- Tmux/Screen
- Top/Htop
- SSH/SCP
- Git (interactive rebase, diff, log)
- Midnight Commander
- Ranger file manager
- Python REPL with readline

---

## References

### Specifications

- [ECMA-48 (Fifth Edition)](https://ecma-international.org/publications-and-standards/standards/ecma-48/) - Control Functions for Coded Character Sets
- [DEC VT100 User Manual](https://vt100.net/docs/vt100-ug/) - Original VT100 documentation
- [DEC VT220 Programmer Reference](https://vt100.net/docs/vt220-rm/) - VT220 specifications
- [DEC VT420 Programmer Reference](https://vt100.net/docs/vt420-ug/) - VT420 features
- [xterm Control Sequences](https://invisible-island.net/xterm/ctlseqs/ctlseqs.html) - Comprehensive xterm sequence reference

### Modern Extensions

- [Kitty Keyboard Protocol](https://sw.kovidgoyal.net/kitty/keyboard-protocol/) - Enhanced keyboard event reporting
- [Synchronized Updates](https://gist.github.com/christianparpart/d8a62cc1ab659194337d73e399004036) - DEC mode 2026
- [OSC 8 Hyperlinks](https://gist.github.com/egmontkob/eb114294efbcd5adb1944c9f3cb5feda) - Terminal hyperlink standard
- [Sixel Graphics](https://vt100.net/docs/vt3xx-gp/chapter14.html) - DEC Sixel specification

### Implementation References

- [VTE Crate](https://docs.rs/vte/) - ANSI/VT parser library
- [PyO3](https://pyo3.rs/) - Rust-Python bindings
- par-term-emu-core-rust source:
  - Terminal core: `src/terminal/mod.rs`
  - Sequence handlers: `src/terminal/sequences/` (csi.rs, esc.rs, osc.rs, dcs.rs)
  - Character writing: `src/terminal/write.rs`
  - Screen buffer: `src/grid.rs`
  - Python bindings: `src/python_bindings/`

---

## See Also

- [config_checklist.md](config_checklist.md) - Configuration options reference
- [ADVANCED_FEATURES.md](ADVANCED_FEATURES.md) - Advanced features guide
- [ARCHITECTURE.md](ARCHITECTURE.md) - System architecture
- [README.md](../README.md) - Project overview
