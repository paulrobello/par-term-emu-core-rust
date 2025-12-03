# VT Technical Reference

**Comprehensive technical reference with implementation details, compatibility matrices, and behavior specifications.**

This document provides detailed VT terminal sequence support information including:
- Implementation details (which Rust modules handle each sequence)
- Compatibility matrices for VT100/VT220/VT320/VT420/VT520
- Detailed parameter handling and edge cases
- Known limitations and testing information

For a quick sequence lookup, see [VT_SEQUENCES.md](VT_SEQUENCES.md).

## Table of Contents

- [Overview](#overview)
- [CSI Sequences](#csi-sequences)
- [ESC Sequences](#esc-sequences)
- [OSC Sequences](#osc-sequences)
- [DCS Sequences](#dcs-sequences)
- [APC Sequences](#apc-sequences)
- [Character Handling](#character-handling)
  - [Wide Character Support](#wide-character-support)
  - [Grapheme Cluster Support](#grapheme-cluster-support)
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
- ✅ **VT520** - Conformance level control, bell volume control
- ✅ **xterm** - Modern extensions (256-color, true color, mouse, etc.)
- ✅ **Modern protocols** - Kitty keyboard, Kitty graphics, iTerm2 images, synchronized updates, OSC 133

### Implementation Location

The terminal implementation uses a modular structure:

**Primary directory:** `src/terminal/`

**Sequence handlers** (in `src/terminal/sequences/`):
- `csi.rs` - CSI sequence handler (`csi_dispatch_impl()`)
- `esc.rs` - ESC sequence handler (`esc_dispatch_impl()`)
- `osc.rs` - OSC sequence handler (`osc_dispatch_impl()`)
- `dcs.rs` - DCS and APC sequence handler (`dcs_hook()`, `dcs_put()`, `dcs_unhook()`)

**Core components:**
- `src/terminal/mod.rs` - Terminal core, VTE callbacks, APC to DCS conversion
- `src/terminal/write.rs` - Character writing and text handling
- `src/grid.rs` - Screen buffer and cell grid
- `src/conformance_level.rs` - VT conformance level management

**Graphics support** (in `src/graphics/`):
- `mod.rs` - Unified graphics store and protocol-agnostic representation
- `kitty.rs` - Kitty graphics protocol parser (APC G)
- `iterm.rs` - iTerm2 inline images parser (OSC 1337)
- `animation.rs` - Animation frame and state management
- `placeholder.rs` - Unicode placeholder support for Kitty virtual placements
- `src/sixel.rs` - Sixel graphics parser (DCS q)

**Unicode and grapheme support:**
- `src/grapheme.rs` - Grapheme cluster detection, emoji sequences, variation selectors, ZWJ handling

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

### Color Stack Operations (xterm)

| Sequence | Name | Notes |
|----------|------|-------|
| `CSI # P` | XTPUSHCOLORS | Push fg, bg, underline colors to stack |
| `CSI # Q` | XTPOPCOLORS | Pop colors from stack |

**Notes:**
- Stack stores foreground, background, and underline colors as a tuple
- Stack grows dynamically as needed
- Pop with empty stack leaves colors unchanged

**Implementation:** `csi_dispatch_impl()` in `src/terminal/sequences/csi.rs` (actions 'P' and 'Q' with '#' intermediate)

### Line and Character Editing (VT220)

| Sequence | Name | VT Level | Notes |
|----------|------|----------|-------|
| `CSI n L` | IL (Insert Lines) | VT220 | Param 0→1, respects scroll region |
| `CSI n M` | DL (Delete Lines) | VT220 | Param 0→1, respects scroll region |
| `CSI n @` | ICH (Insert Characters) | VT220 | Param 0→1, shifts line right |
| `CSI n P` | DCH (Delete Characters) | VT220 | Param 0→1, shifts line left (see note) |

**Note:** `CSI P` without '#' intermediate is DCH. With '#' intermediate (`CSI # P`), it's XTPUSHCOLORS (see Color Stack Operations above).

**Line Editing Behavior:**
- IL/DL only affect rows within scroll region
- New/revealed lines are blank with default attributes
- Respects left/right margins when DECLRMM is enabled

### Rectangle Operations (VT420)

| Sequence | Name | VT Level | Notes |
|----------|------|----------|-------|
| `CSI Pc;Pt;Pl;Pb;Pr $ x` | DECFRA (Fill Rectangle) | VT420 | Fill area with character Pc |
| `CSI Pts;Pls;Pbs;Prs;Pps;Ptd;Pld $ v` | DECCRA (Copy Rectangle) | VT420 | Copy rectangular region |
| `CSI Pt;Pl;Pb;Pr $ z` | DECERA (Erase Rectangle) | VT420 | Unconditional erase (ignores protection) |
| `CSI Pt;Pl;Pb;Pr $ {` | DECSERA (Selective Erase) | VT420 | Selective erase (respects protection) |
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

**Underline Color (xterm):**
```
CSI 58 ; 2 ; r ; g ; b m    - Set underline color to RGB
CSI 58 ; 5 ; n m            - Set underline color to palette index n
CSI 59 m                     - Reset underline color (use foreground)
```

**Default Colors:**
```
CSI 39 m    - Default foreground
CSI 49 m    - Default background
```

**Implementation:** Extended color parsing in `csi_dispatch()`

**Color Parsing Notes:**
- Supports both colon (`:`) and semicolon (`;`) separators for sub-parameters
- 256-color palette: 0-15 (standard), 16-231 (6×6×6 cube), 232-255 (grayscale)
- True color uses 8-bit RGB values (0-255)
- Underline color (SGR 58) is independent from foreground/background colors
- When underline color is not set, foreground color is used for underline rendering

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
- DECSCA handler in `src/terminal/sequences/csi.rs` (CSI ? Ps " q)
- SPA/EPA handlers in `src/terminal/sequences/esc.rs` (ESC V/W)
- Character printing applies guarded flag in `src/terminal/write.rs`
- Grid selective erase method `erase_rectangle()` in `src/grid.rs`
- Grid unconditional erase method `erase_rectangle_unconditional()` in `src/grid.rs`

**Sequence Examples:**
```
CSI ? 1 " q        Enable protection (or ESC V for SPA)
Hello World        (these chars are protected)
CSI ? 0 " q        Disable protection (or ESC W for EPA)
Normal text        (these chars are NOT protected)
CSI 1 ; 1 ; 5 ; 20 $ {    DECSERA - only erases unprotected text
```

**Alternative Sequences:**
- `ESC V` (SPA - Start of Protected Area) - Enable character protection
- `ESC W` (EPA - End of Protected Area) - Disable character protection

See also: [ESC Sequences](#esc-sequences) for ESC V/W details

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

**Response:** Varies based on conformance level (see DECSCL)

**Default Response (VT520):** `CSI ? 65 ; 1 ; 4 ; 6 ; 9 ; 15 ; 22 c`

**Terminal IDs:**
- `1` - VT100
- `62` - VT220
- `63` - VT320
- `64` - VT420
- `65` - VT520 (default)

**Feature Codes:**
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

**Note:** Mode query returns state: 0 (not recognized), 1 (set), 2 (reset), 3 (permanently set), 4 (permanently reset)

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
| 22 | Push title | None | Push current title to stack |
| 23 | Pop title | None | Pop title from stack and apply |
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
| `CSI > flags u` | Push current flags and set new flags |
| `CSI < count u` | Pop flags (count times, default 1) |

**Notes:**
- Separate stacks for primary and alternate screens
- Stack grows dynamically as needed
- Flags control event reporting and key disambiguation
- Pop with no saved state leaves flags unchanged

**Implementation:** `csi_dispatch_impl()` in `src/terminal/sequences/csi.rs`

### VT520 Conformance Level Control

#### DECSCL - Set Conformance Level

`CSI Pl ; Pc " p` - Set terminal conformance level

**Parameters:**
- `Pl` - Conformance level:
  - `1` or `61` - VT100
  - `2` or `62` - VT220
  - `3` or `63` - VT320
  - `4` or `64` - VT420
  - `5` or `65` - VT520 (default)
- `Pc` - 8-bit control mode:
  - `0` - 7-bit controls
  - `1` or `2` - 8-bit controls (default: 2)

**Notes:**
- Changes the terminal's conformance level, affecting which sequences are recognized
- The 8-bit control mode parameter is parsed but not enforced (modern terminals support 8-bit regardless)
- Device Attributes (DA) response reflects the current conformance level
- Default conformance level is VT520

**Implementation:**
- Handler in `src/terminal/sequences/csi.rs`
- Conformance level types in `src/conformance_level.rs`

**Example:**
```
CSI 62 ; 2 " p    # Set to VT220 with 8-bit controls
CSI 5 " p         # Set to VT520 (short form)
CSI 65 " p        # Set to VT520 (long form)
```

**See Also:** `src/conformance_level.rs` for feature-level support checking

#### DECSWBV - Set Warning-Bell Volume

`CSI Ps SP t` - Set warning bell volume (VT520)

**Parameters:**
- `Ps` - Volume level:
  - `0` - Off
  - `1` - Low
  - `2-4` - Medium levels
  - `5-8` - High levels

**Notes:**
- Controls the volume of the warning bell
- Values above 8 are clamped to 8
- Default volume is 4 (moderate)

**Implementation:** `csi_dispatch_impl()` in `src/terminal/sequences/csi.rs`

**Example:**
```
CSI 0 SP t    # Turn off warning bell
CSI 4 SP t    # Set to medium volume
CSI 8 SP t    # Set to maximum volume
```

#### DECSMBV - Set Margin-Bell Volume

`CSI Ps SP u` - Set margin bell volume (VT520)

**Parameters:**
- `Ps` - Volume level:
  - `0` - Off
  - `1` - Low
  - `2-4` - Medium levels
  - `5-8` - High levels

**Notes:**
- Controls the volume of the margin bell
- Values above 8 are clamped to 8
- Default volume is 4 (moderate)
- Independent from warning bell volume

**Implementation:** `csi_dispatch_impl()` in `src/terminal/sequences/csi.rs`

**Example:**
```
CSI 0 SP u    # Turn off margin bell
CSI 4 SP u    # Set to medium volume
CSI 8 SP u    # Set to maximum volume
```

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
| `ESC V` | SPA | VT420 | Start of Protected Area (enable char protection) |
| `ESC W` | EPA | VT420 | End of Protected Area (disable char protection) |

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
| `OSC 21 ; title ST` | Push title to stack | Pushes title or current if empty |
| `OSC 22 ST` | Pop window title from stack | Restores previously pushed title |
| `OSC 23 ST` | Pop icon title from stack | Same as OSC 22 (no distinction) |

**Note:** Title stack operations are also available via XTWINOPS (CSI 22 t / CSI 23 t).

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

### Notifications and Progress

#### iTerm2 Notifications

`OSC 9 ; message ST`

**Implementation:** `osc_dispatch_impl()` in `src/terminal/sequences/osc.rs`
**Security:** Can be blocked via `disable_insecure_sequences`

#### Progress Bar (OSC 9;4)

`OSC 9 ; 4 ; state [; progress] ST` - ConEmu/Windows Terminal style progress reporting

**Implementation:**
- OSC handler in `src/terminal/sequences/osc.rs` (`handle_osc9_progress()`)
- Progress types in `src/terminal/progress.rs`

**States:**
| State | Code | Progress Required | Description |
|-------|------|-------------------|-------------|
| Hidden | 0 | No | Hide progress bar |
| Normal | 1 | Yes (0-100) | Normal progress display |
| Indeterminate | 2 | No | Busy/unknown progress indicator |
| Warning | 3 | Yes (0-100) | Operation with potential issues |
| Error | 4 | Yes (0-100) | Failed operation |

**Examples:**
```
OSC 9 ; 4 ; 1 ; 50 ST    # Set progress to 50%
OSC 9 ; 4 ; 0 ST         # Hide progress bar
OSC 9 ; 4 ; 2 ST         # Show indeterminate progress
OSC 9 ; 4 ; 3 ; 75 ST    # Show warning state at 75%
OSC 9 ; 4 ; 4 ; 100 ST   # Show error state at 100%
```

**Features:**
- Progress values automatically clamped to 0-100
- Can be used for file transfers, compilation, deployments
- Terminal UI can display in tab bar, title bar, or dedicated UI
- Indeterminate state for operations with unknown duration

**Python API:**
```python
# Get progress state
progress = terminal.progress_bar()
has_progress = terminal.has_progress()
value = terminal.progress_value()
state = terminal.progress_state()

# Set progress
terminal.set_progress(ProgressState.Normal, 50)
terminal.clear_progress()
```

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

### Color Queries and Modification

#### Query Colors

| Sequence | Query | Response |
|----------|-------|----------|
| `OSC 10 ; ? ST` | Default foreground | `OSC 10 ; rgb:rrrr/gggg/bbbb ST` |
| `OSC 11 ; ? ST` | Default background | `OSC 11 ; rgb:rrrr/gggg/bbbb ST` |
| `OSC 12 ; ? ST` | Cursor color | `OSC 12 ; rgb:rrrr/gggg/bbbb ST` |

**Format:** 16-bit RGB values (0000-ffff) per component

**Example response:** `OSC 10 ; rgb:ffff/ffff/ffff ST` (white)

#### Set Colors

| Sequence | Operation | Security |
|----------|-----------|----------|
| `OSC 4 ; idx ; colorspec ST` | Set ANSI palette color | Requires insecure sequences enabled |
| `OSC 10 ; colorspec ST` | Set default foreground | Requires insecure sequences enabled |
| `OSC 11 ; colorspec ST` | Set default background | Requires insecure sequences enabled |
| `OSC 12 ; colorspec ST` | Set cursor color | Requires insecure sequences enabled |
| `OSC 104 ST` | Reset all ANSI colors | Requires insecure sequences enabled |
| `OSC 104 ; idx ST` | Reset specific ANSI color | Requires insecure sequences enabled |
| `OSC 110 ST` | Reset default foreground | Requires insecure sequences enabled |
| `OSC 111 ST` | Reset default background | Requires insecure sequences enabled |
| `OSC 112 ST` | Reset cursor color | Requires insecure sequences enabled |

**Color specification formats:**
- `rgb:RR/GG/BB` (hex values, case-insensitive)
- `#RRGGBB` (hex format)

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

### iTerm2 Inline Images (OSC 1337)

`OSC 1337 ; File=name=<base64>;size=<bytes>;inline=1:<base64-data> ST`

**Implementation:**
- OSC handler in `src/terminal/sequences/osc.rs`
- iTerm2 parser in `src/graphics/iterm.rs`
- Graphics store in `src/graphics/mod.rs`

**Parameters:**

| Parameter | Required | Description |
|-----------|----------|-------------|
| `name` | No | Filename (base64-encoded) |
| `size` | No | File size in bytes |
| `width` | No | Display width (auto, Npx, N%, Ncells) |
| `height` | No | Display height (auto, Npx, N%, Ncells) |
| `preserveAspectRatio` | No | 0=stretch, 1=preserve (default=1) |
| `inline` | Yes | Must be `1` for display |

**Supported Formats:**
- PNG (detected automatically)
- JPEG (detected automatically)
- GIF (static only, first frame)
- Base64-encoded image data follows colon (`:`)

**Width/Height Specifications:**
- `auto` - Original dimensions
- `Npx` - Exact pixels (e.g., `100px`)
- `N%` - Percentage of terminal width/height (e.g., `50%`)
- `Ncells` - Terminal cells (e.g., `10cells`)

**Example:**
```
OSC 1337 ; File=inline=1:iVBORw0KGgoAAAA... ST
```

**Features:**
- Automatic image format detection (PNG, JPEG, GIF)
- Dimension specification in multiple units
- Aspect ratio preservation
- Base64 decoding with padding tolerance

**Security:** Can be blocked via `disable_insecure_sequences`

**Note:** Graphics are converted to RGBA pixel data and stored in the unified `GraphicsStore` alongside Sixel and Kitty graphics.

---

## DCS Sequences

DCS (Device Control String) sequences follow: `ESC P ... ESC \`

**Implementation:** `src/terminal/sequences/dcs.rs`

### Sixel Graphics (DCS q)

`DCS Pa ; Pb ; Ph q ... ST`

**Implementation:**
- DCS handlers in `src/terminal/sequences/dcs.rs` (`dcs_hook()`, `dcs_put()`, `dcs_unhook()`)
- Sixel parser in `src/sixel.rs`
- Graphics store in `src/graphics/mod.rs`

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

**Resource Limits:**
- Sixel graphics are subject to per-terminal limits to prevent pathological memory usage:
  - Default: 1024x1024 pixels, max repeat count 10,000, max 256 graphics
  - Hard ceilings: 4096x4096 pixels, repeat count ≤ 10,000, max 1024 graphics
- Limits can be tuned via:
  - Rust: `Terminal::set_sixel_limits(max_width, max_height, max_repeat)` and `Terminal::set_max_sixel_graphics(max_count)`
  - Python: `Terminal.set_sixel_limits(...)` and `Terminal.set_max_sixel_graphics(...)`

**Security:** Can be blocked via `disable_insecure_sequences`

### Kitty Graphics Protocol (APC G)

`APC G <key>=<value>,<key>=<value>;<base64-data> ST`

**Note:** The terminal converts APC sequences (`ESC _`) to DCS sequences (`ESC P`) internally since VTE ignores APC.

**Implementation:**
- APC to DCS conversion in `src/terminal/mod.rs`
- DCS handler in `src/terminal/sequences/dcs.rs` (action 'G')
- Kitty parser in `src/graphics/kitty.rs`
- Graphics store in `src/graphics/mod.rs`

#### Actions

| Action | Code | Description |
|--------|------|-------------|
| Transmit | `a=t` | Transmit image data (store only, no display) |
| Transmit and Display | `a=T` | Transmit and display image |
| Query | `a=q` | Query terminal graphics support |
| Display | `a=p` | Display previously transmitted image |
| Delete | `a=d` | Delete images by ID or position |
| Frame | `a=f` | Add animation frame to image |
| Animation Control | `a=a` | Control animation playback |

#### Transmission Format

| Format | Code | Description |
|--------|------|-------------|
| PNG | `f=100` | PNG image data (default) |
| RGB/RGBA | `f=24/32` | Raw RGB(A) pixel data |
| File | `t=f` | Load from file path |
| Temporary File | `t=t` | Load from temporary file |

#### Parameters

**Image Identification:**
- `i=<id>` - Image ID for reuse
- `I=<placement_id>` - Placement ID (unique instance)

**Dimensions:**
- `s=<width>` - Source width in pixels
- `v=<height>` - Source height in pixels
- `c=<cols>` - Display width in terminal cells
- `r=<rows>` - Display height in terminal cells

**Positioning:**
- `X=<offset>` - Left edge offset in pixels
- `Y=<offset>` - Top edge offset in pixels
- `P=<parent_id>` - Parent image ID for relative positioning
- `Q=<parent_placement>` - Parent placement ID for relative positioning

**Animation:**
- `z=<index>` - Frame index (0-based)
- `g=<gap>` - Gap in milliseconds before next frame
- `s=<composition>` - Composition mode (0=alpha blend, 1=overwrite)

**Chunking:**
- `m=1` - More chunks follow
- `m=0` - Last chunk (default)

**Other:**
- `U=1` - Virtual placement (Unicode placeholder mode)
- `o=z` - Compression (z=zlib)

#### Features

**Image Reuse:**
- Images transmitted with `a=t` are stored by ID
- Subsequent placements use `a=p,i=<id>` to reuse pixel data
- Reduces memory usage for multiple instances
- Managed by `GraphicsStore` with Arc-based pixel sharing

**Animation Support:**
- Multi-frame animations via `a=f` action
- Frame timing control with `g=<ms>` parameter
- Composition modes: alpha blend (0) or overwrite (1)
- Animation state stored in `GraphicsStore`
- Playback controlled via `a=a` action

**Unicode Placeholders:**
- Virtual placements (`U=1`) create template images stored but not displayed
- Terminal automatically inserts U+10EEEE placeholder characters into grid
- Metadata encoded in cell colors and combining characters:
  - Foreground RGB: image_id (lower 24 bits)
  - Underline RGB: placement_id (full 24 bits)
  - Diacritics (combining characters): row/column position and MSB of image_id
    - First diacritic: row (0-63)
    - Second diacritic: column (0-63)
    - Third diacritic: MSB of image_id (0-63, optional)
- Diacritics use special Unicode combining marks (64 different marks for values 0-63)
- Enables inline image display in text flow with inheritance optimization
- Frontend looks up virtual placement using encoded IDs
- See `src/graphics/placeholder.rs` for encoding/decoding implementation

**Chunked Transmission:**
- Large images split across multiple sequences
- `m=1` indicates more chunks follow
- Final chunk uses `m=0` (default)

**Query Response:**
```
APC G i=<id>;OK ST
```

**Resource Limits:**
- Maximum image dimensions enforced by `GraphicsLimits`
- Graphics count limited to prevent memory exhaustion
- Oldest graphics dropped when limit reached
- See `GraphicsLimits` in `src/graphics/mod.rs`

**Implementation Details:**
- RGBA pixel data stored with Arc for sharing
- Position tracked as (col, row) in terminal
- Scroll adjustments automatically applied
- Graphics scrolled off-screen moved to scrollback
- Dropped count tracked for debugging

---

## APC Sequences

APC (Application Program Command) sequences follow: `ESC _ ... ESC \`

**Implementation Note:** The VTE parser library ignores APC sequences, so the terminal converts them to DCS sequences internally before parsing. This conversion happens in `src/terminal/mod.rs`.

### Kitty Graphics Protocol

See [Kitty Graphics Protocol](#kitty-graphics-protocol-apc-g) in the DCS Sequences section above.

**Sequence Format:**
```
APC G <key>=<value>,<key>=<value>;<base64-data> ST
```

**Internal Conversion:**
- `ESC _` → `ESC P` (APC start to DCS start)
- Payload remains unchanged
- `ST` terminator handled by VTE

**Why This Conversion?**
- VTE parser library only processes DCS sequences
- APC sequences are ignored by default
- Kitty graphics protocol specifies APC format
- Conversion enables Kitty compatibility without modifying VTE

**Affected Commands:**
- All Kitty graphics commands (`APC G ...`)

**Implementation:** `src/terminal/mod.rs` (process method, APC to DCS conversion)

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

**Implementation:** Character width detection and printing in `src/terminal/write.rs`, grapheme utilities in `src/grapheme.rs`

**Features:**
- Detects wide characters (East Asian Width property)
- Allocates 2 columns for wide characters (CJK, emoji)
- Uses spacer cells for wide character continuations
- Proper handling of wide characters at line boundaries
- Full grapheme cluster support for complex emoji sequences

**Width Detection:**
- Uses Unicode `EastAsianWidth` property
- Wide (W) and Fullwidth (F) characters occupy 2 cells
- Combining marks treated as width 0
- Special handling for emoji with modifiers and ZWJ sequences

### Grapheme Cluster Support

**Implementation:** `src/grapheme.rs`

The terminal provides comprehensive support for complex Unicode grapheme clusters:

**Variation Selectors:**
- U+FE0E (VS15) - Text style rendering
- U+FE0F (VS16) - Emoji style rendering
- Example: ⚠ (U+26A0) + U+FE0F = ⚠️ (colored emoji)

**Zero Width Joiner (ZWJ) Sequences:**
- U+200D combines multiple emoji into single glyphs
- Examples:
  - 👨‍👩‍👧‍👦 = MAN + ZWJ + WOMAN + ZWJ + GIRL + ZWJ + BOY
  - 🏳️‍🌈 = WHITE FLAG + VS16 + ZWJ + RAINBOW
- Always rendered as wide (2 cells)

**Skin Tone Modifiers (Fitzpatrick):**
- U+1F3FB through U+1F3FF (5 skin tone levels)
- Applied to emoji that support skin tone variation
- Example: 👋🏽 = WAVING HAND + MEDIUM SKIN TONE
- Always rendered as wide (2 cells)

**Regional Indicator Symbols:**
- U+1F1E6 through U+1F1FF (26 indicators for A-Z)
- Used in pairs to form flag emoji
- Examples:
  - 🇺🇸 = U+1F1FA (🇺) + U+1F1F8 (🇸)
  - 🇬🇧 = U+1F1EC (🇬) + U+1F1E7 (🇧)
- Always rendered as wide (2 cells)

**Combining Marks:**
- Diacritics and accents (U+0300-U+036F)
- Combining marks for symbols (U+20D0-U+20FF)
- Hebrew and Arabic combining marks
- Width 0 (overlay on previous character)

**Grapheme Width Determination:**
- Regional indicator pairs: always 2 cells
- ZWJ sequences: always 2 cells
- Emoji with skin tone modifiers: always 2 cells
- Emoji with variation selector U+FE0F: typically 2 cells
- Fallback to `unicode-width` crate for other cases

### Auto-Wrap Mode (DECAWM)

**Implementation:** Character printing and line wrapping logic in `src/terminal/write.rs`

**Behavior:**
- When enabled (default): Characters at right margin wrap to next line
- When disabled: Characters at right margin overwrite last column
- Delayed wrap: Wrap occurs when next character is written
- Wrap flag persists across cursor movements

### Insert Mode (IRM)

**Implementation:** Character insertion and replacement logic in `src/terminal/write.rs`

**Behavior:**
- When enabled: New characters shift existing characters right
- When disabled (default): New characters replace existing characters
- Shifted characters that exceed right margin are lost

### Tab Stops

**Implementation:**
- Tab handling in character printing (`src/terminal/write.rs`)
- HTS (Set Tab Stop) in `esc_dispatch_impl()` (`src/terminal/sequences/esc.rs`)
- TBC (Tab Clear), CHT (Forward Tab), CBT (Backward Tab) in `csi_dispatch_impl()` (`src/terminal/sequences/csi.rs`)

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
| Character protection | ✅ Full | DECSCA (CSI ? Ps " q), SPA/EPA (ESC V/W), selective erase |

### VT520 Compatibility

| Feature Category | Support | Notes |
|------------------|---------|-------|
| Conformance level control | ✅ Full | DECSCL (set level), DA response varies by level |
| Bell volume control | ✅ Full | DECSWBV (warning bell), DECSMBV (margin bell) |
| Device Attributes | ✅ Full | Reports VT520 (id=65) by default |

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
| Kitty Graphics | ✅ Full | `src/graphics/kitty.rs` | APC G protocol, animations, image reuse, Unicode placeholders |
| iTerm2 Inline Images | ✅ Full | `src/graphics/iterm.rs` | OSC 1337 File protocol |
| Synchronized Updates | ✅ Full | Mode 2026 | Flicker-free rendering |
| OSC 8 Hyperlinks | ✅ Full | `src/terminal/sequences/osc.rs` | With deduplication |
| OSC 52 Clipboard | ✅ Full | `src/terminal/sequences/osc.rs` | Read/write with security controls |
| OSC 133 Shell Integration | ✅ Full | `src/terminal/sequences/osc.rs` | Prompt/command/output markers |
| OSC 7 Directory Tracking | ✅ Full | `src/terminal/sequences/osc.rs` | URL-encoded paths |
| OSC 9;4 Progress Bar | ✅ Full | `src/terminal/sequences/osc.rs`, `src/terminal/progress.rs` | ConEmu/Windows Terminal style progress |
| Underline styles | ✅ Full | `src/terminal/sequences/csi.rs` | 6 different styles |

### Unicode Support

| Feature | Support | Implementation | Notes |
|---------|---------|----------------|-------|
| Wide characters (CJK) | ✅ Full | `src/terminal/write.rs` | 2-cell width detection |
| Emoji (base) | ✅ Full | `src/grapheme.rs` | Width detection via unicode-width |
| Variation selectors | ✅ Full | `src/grapheme.rs` | U+FE0E (text), U+FE0F (emoji) |
| ZWJ sequences | ✅ Full | `src/grapheme.rs` | Family emoji, flag combinations |
| Skin tone modifiers | ✅ Full | `src/grapheme.rs` | Fitzpatrick types 1-5 |
| Regional indicators | ✅ Full | `src/grapheme.rs` | Flag emoji (pair detection) |
| Combining marks | ✅ Full | `src/grapheme.rs` | Diacritics, accents (width 0) |
| Grapheme clusters | ✅ Full | `src/grapheme.rs` | Complex emoji rendering |

### Graphics Protocol Support

| Protocol | Format | Implementation | Features |
|----------|--------|----------------|----------|
| Sixel | DCS q | `src/sixel.rs`, `src/graphics/mod.rs` | Palette, repeat, raster attributes |
| Kitty Graphics | APC G | `src/graphics/kitty.rs` | Animations, image reuse, Unicode placeholders |
| iTerm2 Inline | OSC 1337 | `src/graphics/iterm.rs` | PNG, JPEG, GIF, dimension control |

**Unified Architecture:**
- All protocols normalized to RGBA pixel data
- Stored in `GraphicsStore` with position tracking
- Automatic scroll adjustment and scrollback support
- Resource limits prevent memory exhaustion
- Arc-based pixel sharing for Kitty image reuse

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
- [Kitty Graphics Protocol](https://sw.kovidgoyal.net/kitty/graphics-protocol/) - APC-based graphics with animations and image reuse
- [Kitty Unicode Placeholders](https://sw.kovidgoyal.net/kitty/graphics-protocol/#unicode-placeholders) - Virtual placement with U+10EEEE
- [iTerm2 Inline Images](https://iterm2.com/documentation-images.html) - OSC 1337 inline image protocol
- [Synchronized Updates](https://gist.github.com/christianparpart/d8a62cc1ab659194337d73e399004036) - DEC mode 2026
- [OSC 8 Hyperlinks](https://gist.github.com/egmontkob/eb114294efbcd5adb1944c9f3cb5feda) - Terminal hyperlink standard
- [OSC 9;4 Progress Bar](https://conemu.github.io/en/AnsiEscapeCodes.html#ConEmu_specific_OSC) - ConEmu/Windows Terminal progress reporting
- [Sixel Graphics](https://vt100.net/docs/vt3xx-gp/chapter14.html) - DEC Sixel specification
- [OSC 52 Clipboard](https://chromium.googlesource.com/apps/libapps/+/HEAD/nassh/doc/FAQ.md#Is-OSC-52-aka-clipboard-operations_supported) - Clipboard manipulation protocol
- [Unicode Standard](https://www.unicode.org/versions/latest/) - Unicode character properties and emoji specifications

### Implementation References

- [VTE Crate](https://docs.rs/vte/) - ANSI/VT parser library
- [PyO3](https://pyo3.rs/) - Rust-Python bindings
- [image crate](https://docs.rs/image/) - Image decoding (PNG, JPEG, GIF)
- [unicode-width crate](https://docs.rs/unicode-width/) - Unicode character width detection
- par-term-emu-core-rust source:
  - Terminal core: `src/terminal/mod.rs`
  - Sequence handlers: `src/terminal/sequences/` (csi.rs, esc.rs, osc.rs, dcs.rs)
  - Character writing: `src/terminal/write.rs`
  - Screen buffer: `src/grid.rs`
  - Graphics:
    - Unified store: `src/graphics/mod.rs`
    - Sixel parser: `src/sixel.rs`
    - Kitty protocol: `src/graphics/kitty.rs`
    - iTerm2 protocol: `src/graphics/iterm.rs`
    - Animation support: `src/graphics/animation.rs`
    - Unicode placeholders: `src/graphics/placeholder.rs`
  - Unicode support:
    - Grapheme utilities: `src/grapheme.rs`
  - Conformance levels: `src/conformance_level.rs`
  - Progress bar support: `src/terminal/progress.rs`
  - Python bindings: `src/python_bindings/`

---

## See Also

- [ADVANCED_FEATURES.md](ADVANCED_FEATURES.md) - Advanced features guide (OSC 52, OSC 133, Sixel, etc.)
- [ARCHITECTURE.md](ARCHITECTURE.md) - System architecture and design
- [README.md](../README.md) - Project overview and API documentation
- [DOCUMENTATION_STYLE_GUIDE.md](DOCUMENTATION_STYLE_GUIDE.md) - Documentation standards
