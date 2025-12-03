# VT Sequences Reference

**Quick reference guide for supported ANSI/VT escape sequences.**

This is a concise lookup table of supported sequences. For detailed behavior, implementation notes, compatibility matrices, and edge cases, see [VT_TECHNICAL_REFERENCE.md](VT_TECHNICAL_REFERENCE.md).

**Compatibility Level:** VT100/VT220/VT320/VT420/VT520 + xterm extensions + modern protocols (Kitty keyboard, Kitty graphics, Sixel, iTerm2 images, OSC 8 hyperlinks, OSC 133 shell integration)

## Table of Contents

- [Cursor Movement](#cursor-movement)
- [Display Control](#display-control)
- [Line/Character Editing](#linecharacter-editing)
- [Rectangle Operations](#rectangle-operations)
- [Scrolling](#scrolling)
- [Colors and Attributes](#colors-and-attributes)
- [Tab Stops](#tab-stops)
- [Terminal Modes](#terminal-modes)
- [Mouse Support](#mouse-support)
- [Advanced Features](#advanced-features)
- [Kitty Keyboard Protocol](#kitty-keyboard-protocol)
- [Device Queries](#device-queries)
- [OSC Sequences](#osc-sequences)
- [DCS Sequences](#dcs-sequences)
- [APC Sequences](#apc-sequences)
- [Control Characters](#control-characters)
- [Reset Sequences](#reset-sequences)

## Cursor Movement

VT100 cursor movement sequences.

- `CSI <n> A` - Cursor up n lines (CUU)
- `CSI <n> B` - Cursor down n lines (CUD)
- `CSI <n> C` - Cursor forward n columns (CUF)
- `CSI <n> D` - Cursor back n columns (CUB)
- `CSI <n> E` - Cursor next line (CNL)
- `CSI <n> F` - Cursor previous line (CPL)
- `CSI <n> G` - Cursor horizontal absolute (CHA)
- `CSI <row> ; <col> H` - Cursor position (CUP)
- `CSI <row> ; <col> f` - Cursor position (HVP - alternative)
- `CSI <n> d` - Line position absolute (VPA)
- `CSI s` - Save cursor position (ANSI.SYS)
- `CSI u` - Restore cursor position (ANSI.SYS)
- `ESC 7` - Save cursor (DECSC)
- `ESC 8` - Restore cursor (DECRC)

## Display Control

VT100 screen clearing and erasing sequences.

### Erase in Display (ED)

`CSI <n> J`

- `n=0` - Clear from cursor to end
- `n=1` - Clear from beginning to cursor
- `n=2` - Clear entire screen
- `n=3` - Clear entire screen and scrollback

### Erase in Line (EL)

`CSI <n> K`

- `n=0` - Clear from cursor to end of line
- `n=1` - Clear from beginning of line to cursor
- `n=2` - Clear entire line

## Line/Character Editing

VT220 insert/delete operations.

- `CSI <n> L` - Insert n blank lines (IL)
- `CSI <n> M` - Delete n lines (DL)
- `CSI <n> @` - Insert n blank characters (ICH)
- `CSI <n> P` - Delete n characters (DCH)
- `CSI <n> X` - Erase n characters (ECH)

## Rectangle Operations

VT420 advanced text editing operations that work on rectangular regions of the screen. All coordinates are 1-indexed.

- `CSI Pc ; Pt ; Pl ; Pb ; Pr $ x` - DECFRA: Fill rectangle with character `Pc`
- `CSI Pts ; Pls ; Pbs ; Prs ; Pps ; Ptd ; Pld ; Ppd $ v` - DECCRA: Copy rectangular region
- `CSI Pt ; Pl ; Pb ; Pr $ {` - DECSERA: Selective erase (respects protection)
- `CSI Pt ; Pl ; Pb ; Pr $ z` - DECERA: Unconditional erase (ignores protection)
- `CSI Pt ; Pl ; Pb ; Pr ; Ps $ r` - DECCARA: Change attributes in rectangle
- `CSI Pt ; Pl ; Pb ; Pr ; Ps $ t` - DECRARA: Reverse attributes in rectangle
- `CSI Pi ; Pg ; Pt ; Pl ; Pb ; Pr * y` - DECRQCRA: Request rectangle checksum
- `CSI Ps * x` - DECSACE: Set attribute change extent (0/1=stream, 2=rectangle)

> See [VT_TECHNICAL_REFERENCE.md#rectangle-operations](VT_TECHNICAL_REFERENCE.md#rectangle-operations-vt420) for detailed parameter descriptions and behavior.

## Scrolling

VT100/VT220 scrolling operations.

### CSI Scrolling Commands

- `CSI <n> S` - Scroll up n lines (SU)
- `CSI <n> T` - Scroll down n lines (SD)
- `CSI <top> ; <bottom> r` - Set scrolling region (DECSTBM)

### ESC Scrolling Commands

- `ESC M` - Reverse index (RI) - Move cursor up one line, scroll down if at top of scroll region
- `ESC D` - Index (IND) - Move cursor down one line, scroll up if at bottom of scroll region
- `ESC E` - Next line (NEL) - Move to first column of next line, scroll if at bottom

## Colors and Attributes

VT100/ECMA-48 text styling sequences.

### Basic Attributes

- `CSI 0 m` - Reset all attributes (SGR 0)
- `CSI 1 m` - Bold
- `CSI 2 m` - Dim
- `CSI 3 m` - Italic
- `CSI 4 m` - Underline (basic, defaults to straight)
- `CSI 5 m` - Blink
- `CSI 7 m` - Reverse
- `CSI 8 m` - Hidden
- `CSI 9 m` - Strikethrough
- `CSI 53 m` - Overline
- `CSI 55 m` - Not overlined

### Underline Styles

- `CSI 4 : 0 m` - No underline (explicit)
- `CSI 4 : 1 m` - Straight underline (default)
- `CSI 4 : 2 m` - Double underline
- `CSI 4 : 3 m` - Curly underline (spell check, errors)
- `CSI 4 : 4 m` - Dotted underline
- `CSI 4 : 5 m` - Dashed underline

### Attribute Reset

- `CSI 22 m` - Normal intensity (not bold or dim)
- `CSI 23 m` - Not italic
- `CSI 24 m` - Not underlined
- `CSI 25 m` - Not blinking
- `CSI 27 m` - Not reversed
- `CSI 28 m` - Not hidden
- `CSI 29 m` - Not strikethrough

### Basic Colors

- `CSI 30-37 m` - Foreground colors (black, red, green, yellow, blue, magenta, cyan, white)
- `CSI 40-47 m` - Background colors
- `CSI 90-97 m` - Bright foreground colors (aixterm)
- `CSI 100-107 m` - Bright background colors (aixterm)

### Extended Colors

- `CSI 38 ; 5 ; <n> m` - 256-color foreground (0-255)
- `CSI 48 ; 5 ; <n> m` - 256-color background (0-255)
- `CSI 38 ; 2 ; <r> ; <g> ; <b> m` - RGB/true color foreground
- `CSI 48 ; 2 ; <r> ; <g> ; <b> m` - RGB/true color background
- `CSI 58 ; 2 ; <r> ; <g> ; <b> m` - RGB underline color
- `CSI 58 ; 5 ; <n> m` - 256-color underline color
- `CSI 59 m` - Reset underline color (use foreground)

### Default Colors

- `CSI 39 m` - Default foreground color
- `CSI 49 m` - Default background color

## Tab Stops

VT100 tab stop management.

- `ESC H` - Set tab stop at current column (HTS)
- `CSI <n> g` - Tab clear (TBC)
  - `n=0` - Clear tab at current column
  - `n=3` - Clear all tabs
- `CSI <n> I` - Cursor forward tabulation (CHT)
- `CSI <n> Z` - Cursor backward tabulation (CBT)

## Terminal Modes

DEC Private Mode sequences.

### Mode Setting

- `CSI ? <n> h` - Set mode
- `CSI ? <n> l` - Reset mode

### Common Modes

- `?1` - Application cursor keys (DECCKM)
- `?5` - Reverse video (DECSCNM)
- `?6` - Origin mode (DECOM)
- `?7` - Auto wrap mode (DECAWM)
- `?25` - Show/hide cursor (DECTCEM)
- `?47` - Alternate screen buffer
- `?69` - Enable left/right margins (DECLRMM)
- `?1047` - Alternate screen buffer (alternate)
- `?1048` - Save/restore cursor
- `?1049` - Save cursor and use alternate screen

### Standard Modes

- `4` - Insert mode (IRM)
- `20` - Line feed/new line mode (LNM)

## Mouse Support

xterm mouse tracking modes and encodings.

### Tracking Modes

- `CSI ? 1000 h/l` - Normal mouse tracking
- `CSI ? 1002 h/l` - Button event mouse tracking
- `CSI ? 1003 h/l` - Any event mouse tracking

### Encoding Modes

- `CSI ? 1005 h/l` - UTF-8 mouse encoding
- `CSI ? 1006 h/l` - SGR mouse encoding
- `CSI ? 1015 h/l` - URXVT mouse encoding

## Advanced Features

Modern terminal features and VT520 extensions.

**Modern protocols:**
- `CSI ? 1004 h/l` - Focus tracking (send CSI I/O on focus in/out)
- `CSI ? 2004 h/l` - Bracketed paste mode (wrap pasted text)
- `CSI ? 2026 h/l` - Synchronized updates (flicker-free rendering)

**VT520 features:**
- `CSI Ps SP u` - Set Margin-Bell Volume (DECSMBV, Ps = 0-8)
- `CSI Ps SP t` - Set Warning-Bell Volume (DECSWBV, Ps = 0-8)
- `CSI Pl ; Pc " p` - Set Conformance Level (DECSCL, Pl = 61-65 for VT100-VT520, Pc = 0/2 for 8-bit controls)

**Character protection:**
- `ESC V` / `ESC W` - Start/End Protected Area (SPA/EPA)
- `CSI ? Ps " q` - Select Character Protection Attribute (DECSCA, Ps: 0/2=unprotected, 1=protected)

**Color stack:**
- `CSI # P` - Push current colors (XTPUSHCOLORS)
- `CSI # Q` - Pop colors from stack (XTPOPCOLORS)

> See [VT_TECHNICAL_REFERENCE.md#modern-extensions](VT_TECHNICAL_REFERENCE.md#modern-extensions) for detailed behavior and VT520 conformance level effects.

## Kitty Keyboard Protocol

Progressive enhancement for keyboard handling with flags for disambiguation and event reporting.

- `CSI = flags ; mode u` - Set keyboard protocol (mode: 0=disable, 1=set, 2=lock, 3=report)
  - Flags (bitmask): 1=disambiguate, 2=report events, 4=alt keys, 8=all keys, 16=text
- `CSI ? u` - Query current flags → Response: `CSI ? flags u`
- `CSI > flags u` - Push current flags and set new
- `CSI < count u` - Pop flags from stack

> See [VT_TECHNICAL_REFERENCE.md#kitty-keyboard-protocol](VT_TECHNICAL_REFERENCE.md#kitty-keyboard-protocol) for detailed flag behavior and screen buffer handling.

## Device Queries

VT100/VT220 device information requests.

- `CSI 5 n` - Device Status Report (DSR) → `CSI 0 n` (ready)
- `CSI 6 n` - Cursor Position Report (CPR) → `CSI row ; col R` (1-indexed)
- `CSI c` / `CSI 0 c` - Primary Device Attributes → `CSI ? id ; features c`
- `CSI > c` - Secondary Device Attributes → `CSI > 82 ; 10000 ; 0 c`
- `CSI ? mode $ p` - DEC Private Mode Request (DECRQM) → `CSI ? mode ; state $ y`
- `CSI 0 x` / `CSI 1 x` - Terminal Parameters (DECREQTPARM) → `CSI sol ; 1 ; 1 ; 120 ; 120 ; 1 ; 0 x`
- `CSI 14 t` - Report pixel size → `CSI 4 ; height ; width t`
- `CSI 18 t` - Report text size → `CSI 8 ; rows ; cols t`
- `CSI 22 t` - Save window title to stack
- `CSI 23 t` - Restore window title from stack

### Cursor Style (DECSCUSR)

- `CSI 0 SP q` / `CSI 1 SP q` - Blinking block (default)
- `CSI 2 SP q` - Steady block
- `CSI 3 SP q` - Blinking underline
- `CSI 4 SP q` - Steady underline
- `CSI 5 SP q` - Blinking bar
- `CSI 6 SP q` - Steady bar

### Left/Right Margins (DECSLRM)

- `CSI Pl ; Pr s` - Set left/right margins (requires DECLRMM mode ?69)

> See [VT_TECHNICAL_REFERENCE.md#device-queries](VT_TECHNICAL_REFERENCE.md#device-queries) for detailed response formats and parameter meanings.

## OSC Sequences

Operating System Command sequences for advanced features (format: `OSC Ps ; Pt ST` where ST = ESC\ or BEL).

### Window Title and Directory

- `OSC 0;title ST` - Set window and icon title
- `OSC 2;title ST` - Set window title only
- `OSC 21;title ST` - Push title to stack (or `OSC 21 ST` to push current title)
- `OSC 22 ST` / `OSC 23 ST` - Pop window/icon title from stack
- `OSC 7;file://host/path ST` - Set current working directory (URL-encoded)

### Hyperlinks and Clipboard

- `OSC 8;;url ST text OSC 8;;ST` - Hyperlinks (iTerm2/VTE compatible)
- `OSC 52;c;data ST` - Clipboard operations (base64-encoded)
  - `data` = base64 text to copy
  - `?` = query clipboard (requires `set_allow_clipboard_read(true)`)

### Shell Integration (OSC 133)

iTerm2/VSCode compatible shell integration markers:

- `OSC 133;A ST` - Prompt start
- `OSC 133;B ST` - Command start
- `OSC 133;C ST` - Command executed
- `OSC 133;D;exit_code ST` - Command finished

### Color Operations

**Palette (ANSI colors 0-15):**
- `OSC 4;index;colorspec ST` - Set palette entry (formats: `rgb:RR/GG/BB` or `#RRGGBB`)
- `OSC 104 ST` - Reset all palette colors
- `OSC 104;index ST` - Reset specific palette color

**Default colors:**
- `OSC 10;? ST` / `OSC 10;colorspec ST` / `OSC 110 ST` - Query/set/reset foreground
- `OSC 11;? ST` / `OSC 11;colorspec ST` / `OSC 111 ST` - Query/set/reset background
- `OSC 12;? ST` / `OSC 12;colorspec ST` / `OSC 112 ST` - Query/set/reset cursor

### Notifications

- `OSC 9;message ST` - Simple notification (iTerm2/ConEmu style)
- `OSC 777;notify;title;message ST` - Structured notification (urxvt style)

### Progress Bar (OSC 9;4)

ConEmu/Windows Terminal style progress indicator:

- `OSC 9;4;0 ST` - Hide progress bar
- `OSC 9;4;1;N ST` - Normal progress at N% (0-100)
- `OSC 9;4;2 ST` - Indeterminate/busy indicator
- `OSC 9;4;3;N ST` - Warning progress at N%
- `OSC 9;4;4;N ST` - Error progress at N%

### iTerm2 Inline Images

- `OSC 1337;File=name=<b64>;size=<bytes>;inline=1:<base64 data> ST` - iTerm2 inline images

**Security:** Notifications, color changes, hyperlinks, and Sixel graphics can be disabled via `disable_insecure_sequences`.

> See [VT_TECHNICAL_REFERENCE.md#osc-sequences](VT_TECHNICAL_REFERENCE.md#osc-sequences) for detailed format specifications and security controls.

## DCS Sequences

Device Control String sequences for graphics (format: `DCS params final data ST`).

### Sixel Graphics

`DCS Pa ; Pb ; Ph q data ST`

Full VT340 Sixel graphics support for inline images with configurable limits.

- Color definitions: `#Pc;Pu;Px;Py;Pz`
- Raster attributes: `"Pa;Pb;Ph;Pv`
- Repeat operator: `!Pn s`
- Carriage return: `$`
- New line: `-`
- Sixel data characters: `?` through `~` (ASCII 63-126)
- Up to 256 colors, configurable size limits

**Security:** Can be disabled via `disable_insecure_sequences`. Default limits: 1024×1024 pixels, 256 graphics.

> See [VT_TECHNICAL_REFERENCE.md#sixel-graphics](VT_TECHNICAL_REFERENCE.md#sixel-graphics) for detailed command syntax and [Sixel Graphics Specification](https://vt100.net/docs/vt3xx-gp/chapter14.html).

## APC Sequences

Application Program Command sequences (format: `APC params data ST`).

### Kitty Graphics Protocol

`APC G key=value,key=value;base64-data ST`

Kitty graphics protocol support for modern terminal graphics with animation, composition modes, and advanced features.

- Transmission actions: transmit (t), transmit+display (T), query (q), put (p), delete (d), frame (f), animation control (a)
- Formats: RGB (24), RGBA (32), PNG (100)
- Transmission media: direct (d), file (f), temp file (t), shared memory (s)
- Animation support with frame control and composition modes
- Virtual placements and relative positioning

**Note:** Kitty graphics can also be sent via DCS sequences using `DCS G key=value;data ST` format.

> See [Kitty Graphics Protocol](https://sw.kovidgoyal.net/kitty/graphics-protocol/) for complete specification.

## Control Characters

ASCII control characters.

- `BEL` (0x07) - Bell
- `BS` (0x08) - Backspace
- `HT` (0x09) - Horizontal tab
- `LF` (0x0A) - Line feed
- `CR` (0x0D) - Carriage return
- `ESC` (0x1B) - Escape (starts escape sequences)

## Reset Sequences

- `ESC c` - Reset to initial state (RIS)

## See Also

- [API Reference](API_REFERENCE.md) - Complete Python API documentation
- [VT Technical Reference](VT_TECHNICAL_REFERENCE.md) - Detailed VT compatibility and implementation details
- [Advanced Features](ADVANCED_FEATURES.md) - Feature usage guides
- [xterm Control Sequences](https://invisible-island.net/xterm/ctlseqs/ctlseqs.html) - Official xterm documentation
