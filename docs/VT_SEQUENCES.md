# VT Sequences Reference

Complete reference of supported ANSI/VT escape sequences.

This terminal emulator provides comprehensive VT100/VT220/VT320/VT420 compatibility, matching iTerm2's feature set.

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
- [Control Characters](#control-characters)
- [Reset Sequences](#reset-sequences)

## Cursor Movement

VT100 cursor movement sequences.

- `ESC[<n>A` - Cursor up n lines (CUU)
- `ESC[<n>B` - Cursor down n lines (CUD)
- `ESC[<n>C` - Cursor forward n columns (CUF)
- `ESC[<n>D` - Cursor back n columns (CUB)
- `ESC[<n>E` - Cursor next line (CNL)
- `ESC[<n>F` - Cursor previous line (CPL)
- `ESC[<n>G` - Cursor horizontal absolute (CHA)
- `ESC[<row>;<col>H` - Cursor position (CUP)
- `ESC[<row>;<col>f` - Cursor position (HVP - alternative)
- `ESC[<n>d` - Line position absolute (VPA)
- `ESC[s` - Save cursor position (ANSI.SYS)
- `ESC[u` - Restore cursor position (ANSI.SYS)
- `ESC 7` - Save cursor (DECSC)
- `ESC 8` - Restore cursor (DECRC)

## Display Control

VT100 screen clearing and erasing sequences.

### Erase in Display (ED)

`ESC[<n>J`

- `n=0` - Clear from cursor to end
- `n=1` - Clear from beginning to cursor
- `n=2` - Clear entire screen
- `n=3` - Clear entire screen and scrollback

### Erase in Line (EL)

`ESC[<n>K`

- `n=0` - Clear from cursor to end of line
- `n=1` - Clear from beginning of line to cursor
- `n=2` - Clear entire line

## Line/Character Editing

VT220 insert/delete operations.

- `ESC[<n>L` - Insert n blank lines (IL)
- `ESC[<n>M` - Delete n lines (DL)
- `ESC[<n>@` - Insert n blank characters (ICH)
- `ESC[<n>P` - Delete n characters (DCH)
- `ESC[<n>X` - Erase n characters (ECH)

## Rectangle Operations

VT420 advanced text editing operations that work on rectangular regions of the screen.

### Fill Rectangular Area (DECFRA)

`ESC[<Pc>;<Pt>;<Pl>;<Pb>;<Pr>$x`

- `Pc` - Character code to fill (e.g., 88 for 'X', 42 for '*')
- `Pt` - Top row (1-indexed)
- `Pl` - Left column (1-indexed)
- `Pb` - Bottom row (1-indexed)
- `Pr` - Right column (1-indexed)

Fills rectangle with specified character using current text attributes.

### Copy Rectangular Area (DECCRA)

`ESC[<Pts>;<Pls>;<Pbs>;<Prs>;<Pps>;<Ptd>;<Pld>;<Ppd>$v`

- `Pts`, `Pls`, `Pbs`, `Prs` - Source rectangle (top, left, bottom, right)
- `Pps` - Source page (use 1 for current screen)
- `Ptd`, `Pld` - Destination position (top, left)
- `Ppd` - Destination page (use 1 for current screen)

Copies rectangular region to new location.

### Selective Erase Rectangular Area (DECSERA)

`ESC[<Pt>;<Pl>;<Pb>;<Pr>${`

Selectively erases rectangle (respects character protection attribute).

### Erase Rectangular Area (DECERA)

`ESC[<Pt>;<Pl>;<Pb>;<Pr>$z`

Unconditionally erases rectangle (ignores protection).

### Change Attributes in Rectangular Area (DECCARA)

`ESC[<Pt>;<Pl>;<Pb>;<Pr>;<Ps>$r`

- `Pt`, `Pl`, `Pb`, `Pr` - Rectangle coordinates (top, left, bottom, right)
- `Ps` - SGR attributes to apply:
  - `0` - Reset
  - `1` - Bold
  - `4` - Underline
  - `5` - Blink
  - `7` - Reverse
  - `8` - Hidden

Changes text attributes in rectangle.

### Reverse Attributes in Rectangular Area (DECRARA)

`ESC[<Pt>;<Pl>;<Pb>;<Pr>;<Ps>$t`

- `Ps` - Attributes to reverse (0=all, 1=bold, 4=underline, 5=blink, 7=reverse, 8=hidden)

Toggles attributes in rectangle.

### Request Checksum of Rectangular Area (DECRQCRA)

`ESC[<Pi>;<Pg>;<Pt>;<Pl>;<Pb>;<Pr>*y`

- `Pi` - Request ID
- `Pg` - Page number (use 1 for current screen)
- `Pt`, `Pl`, `Pb`, `Pr` - Rectangle coordinates

Response: `DCS Pi ! ~ xxxx ST` (16-bit checksum in hex)

### Select Attribute Change Extent (DECSACE)

`ESC[<Ps>*x`

- `Ps = 0` or `1` - Stream mode (attributes wrap at line boundaries)
- `Ps = 2` - Rectangle mode (strict rectangular boundaries, default)

Affects how DECCARA and DECRARA apply attributes.

**Use Cases:** Efficient text manipulation in editors (vim, emacs), drawing box characters, clearing specific screen regions without affecting surrounding content, attribute modification without changing text, verification of screen regions via checksums.

## Scrolling

VT100/VT220 scrolling operations.

### CSI Scrolling Commands

- `CSI <n>S` - Scroll up n lines (SU)
- `CSI <n>T` - Scroll down n lines (SD)
- `CSI <top>;<bottom>r` - Set scrolling region (DECSTBM)

### ESC Scrolling Commands

- `ESC M` - Reverse index (RI) - Move cursor up one line, scroll down if at top of scroll region
- `ESC D` - Index (IND) - Move cursor down one line, scroll up if at bottom of scroll region
- `ESC E` - Next line (NEL) - Move to first column of next line, scroll if at bottom

## Colors and Attributes

VT100/ECMA-48 text styling sequences.

### Basic Attributes

- `ESC[0m` - Reset all attributes (SGR 0)
- `ESC[1m` - Bold
- `ESC[2m` - Dim
- `ESC[3m` - Italic
- `ESC[4m` - Underline (basic, defaults to straight)
- `ESC[5m` - Blink
- `ESC[7m` - Reverse
- `ESC[8m` - Hidden
- `ESC[9m` - Strikethrough
- `ESC[53m` - Overline
- `ESC[55m` - Not overlined

### Underline Styles

- `ESC[4:0m` - No underline (explicit)
- `ESC[4:1m` - Straight underline (default)
- `ESC[4:2m` - Double underline
- `ESC[4:3m` - Curly underline (spell check, errors)
- `ESC[4:4m` - Dotted underline
- `ESC[4:5m` - Dashed underline

### Attribute Reset

- `ESC[22m` - Normal intensity (not bold or dim)
- `ESC[23m` - Not italic
- `ESC[24m` - Not underlined
- `ESC[25m` - Not blinking
- `ESC[27m` - Not reversed
- `ESC[28m` - Not hidden
- `ESC[29m` - Not strikethrough

### Basic Colors

- `ESC[30-37m` - Foreground colors (black, red, green, yellow, blue, magenta, cyan, white)
- `ESC[40-47m` - Background colors
- `ESC[90-97m` - Bright foreground colors (aixterm)
- `ESC[100-107m` - Bright background colors (aixterm)

### Extended Colors

- `ESC[38;5;<n>m` - 256-color foreground (0-255)
- `ESC[48;5;<n>m` - 256-color background (0-255)
- `ESC[38;2;<r>;<g>;<b>m` - RGB/true color foreground
- `ESC[48;2;<r>;<g>;<b>m` - RGB/true color background
- `ESC[58;2;<r>;<g>;<b>m` - RGB underline color
- `ESC[58;5;<n>m` - 256-color underline color
- `ESC[59m` - Reset underline color (use foreground)

### Default Colors

- `ESC[39m` - Default foreground color
- `ESC[49m` - Default background color

## Tab Stops

VT100 tab stop management.

- `ESC H` - Set tab stop at current column (HTS)
- `ESC[<n>g` - Tab clear (TBC)
  - `n=0` - Clear tab at current column
  - `n=3` - Clear all tabs
- `ESC[<n>I` - Cursor forward tabulation (CHT)
- `ESC[<n>Z` - Cursor backward tabulation (CBT)

## Terminal Modes

DEC Private Mode sequences.

### Mode Setting

- `ESC[?<n>h` - Set mode
- `ESC[?<n>l` - Reset mode

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

- `ESC[?1000h/l` - Normal mouse tracking
- `ESC[?1002h/l` - Button event mouse tracking
- `ESC[?1003h/l` - Any event mouse tracking

### Encoding Modes

- `ESC[?1005h/l` - UTF-8 mouse encoding
- `ESC[?1006h/l` - SGR mouse encoding
- `ESC[?1015h/l` - URXVT mouse encoding

## Advanced Features

Modern terminal features.

- `ESC[?1004h/l` - Focus tracking
- `ESC[?2004h/l` - Bracketed paste mode
- `ESC[?2026h/l` - Synchronized updates (DEC 2026) - Batch screen updates for flicker-free rendering

### VT520 Features

- `CSI Ps SP u` - Set Margin-Bell Volume (DECSMBV) - Ps = 0-8 (0=off, 1=low, 5-8=high)
- `CSI Ps SP t` - Set Warning-Bell Volume (DECSWBV) - Ps = 0-8 (0=off, 1=low, 5-8=high)
- `CSI Pl ; Pc " p` - Set Conformance Level (DECSCL) - Pl = 61-65 for VT100-VT520, Pc = 0/1/2 for 8-bit mode

### Character Protection

- `ESC V` - Start Protected Area (SPA)
- `ESC W` - End Protected Area (EPA)
- `CSI ? Ps " q` - Select Character Protection Attribute (DECSCA) - Ps = 0/2 (not protected), 1 (protected)

### Color Stack Operations

- `CSI # P` - Push current colors onto stack (XTPUSHCOLORS)
- `CSI # Q` - Pop colors from stack (XTPOPCOLORS)

## Kitty Keyboard Protocol

Progressive enhancement for keyboard handling with flags for disambiguation and event reporting.

### Set Keyboard Protocol Mode

`CSI = flags ; mode u`

**Flags** (bitmask):
- `1` - Disambiguate escape codes
- `2` - Report event types
- `4` - Report alternate keys
- `8` - Report all keys as escape codes
- `16` - Report associated text

**Mode:**
- `0` - Disable
- `1` - Set
- `2` - Lock
- `3` - Report current flags

### Query Keyboard Flags

`CSI ? u`

Response: `CSI ? flags u`

### Push/Pop Flags

- `CSI > flags u` - Push current flags to stack and set new flags
- `CSI < count u` - Pop flags from stack (count times)

**Important:** Flags are maintained separately for main and alternate screen buffers with independent stacks. Flags automatically reset when exiting alternate screen to prevent TUI apps from leaving keyboard in bad state.

## Device Queries

VT100/VT220 device information requests.

### Device Status Report (DSR)

- `CSI 5 n` - Operating status report → Response: `CSI 0 n` (terminal ready)
- `CSI 6 n` - Cursor position report (CPR) → Response: `CSI row ; col R` (1-indexed)

### Device Attributes (DA)

- `CSI c` or `CSI 0 c` - Primary DA → Response: `CSI ? id ; features c` (id: 1=VT100, 62=VT220, 63=VT320, 64=VT420, 65=VT520)
- `CSI > c` - Secondary DA → Response: `CSI > 82 ; 10000 ; 0 c` (82='P' for par-term-emu, version 10000)

### DEC Private Mode Request (DECRQM)

- `CSI ? mode $ p` - Request mode status → Response: `CSI ? mode ; state $ y` (state: 0=not recognized, 1=set, 2=reset, 3=permanently set, 4=permanently reset)

### Terminal Parameters (DECREQTPARM)

- `CSI 0 x` or `CSI 1 x` - Request terminal parameters → Response: `CSI sol ; 1 ; 1 ; 120 ; 120 ; 1 ; 0 x` (sol=2 if param=0, sol=3 if param=1)

### Window Operations (XTWINOPS)

- `CSI 14 t` - Report text area size in pixels → Response: `CSI 4 ; height ; width t`
- `CSI 18 t` - Report text area size in characters → Response: `CSI 8 ; rows ; cols t`
- `CSI 22 t` - Save window title on stack (XTWINOPS)
- `CSI 23 t` - Restore window title from stack (XTWINOPS)

### Cursor Style (DECSCUSR)

- `CSI 0 SP q` or `CSI 1 SP q` - Blinking block (default)
- `CSI 2 SP q` - Steady block
- `CSI 3 SP q` - Blinking underline
- `CSI 4 SP q` - Steady underline
- `CSI 5 SP q` - Blinking bar
- `CSI 6 SP q` - Steady bar

### Left/Right Margins (DECSLRM)

- `CSI Pl ; Pr s` - Set left/right margins (only when DECLRMM ?69 is enabled)

## OSC Sequences

Operating System Command sequences for advanced features.

### Window Title

- `OSC 0;<title>ST` - Set window title (icon + title)
- `OSC 2;<title>ST` - Set window title
- `OSC 21;<title>ST` - Push window title onto stack (XTWINOPS)
- `OSC 22ST` - Pop window title from stack (XTWINOPS)
- `OSC 23ST` - Pop icon title from stack (XTWINOPS)

### Current Working Directory

- `OSC 7;<cwd>ST` - Set current working directory

### Hyperlinks

`OSC 8;;<url>ST`

Full support with clickable TUI rendering (iTerm2/VTE compatible).

### Clipboard Operations

`OSC 52;c;<data>ST`

Works over SSH without X11 (xterm/iTerm2 compatible).

- `<data>` - base64 encoded text to copy to clipboard
- `?` - Query clipboard (requires `set_allow_clipboard_read(true)` for security)
- Empty data clears clipboard

### Shell Integration

`OSC 133;<marker>ST`

iTerm2/VSCode compatible shell integration.

- `A` - Prompt start
- `B` - Command start
- `C` - Command executed
- `D;<exit_code>` - Command finished

### Color Palette Operations

- `OSC 4;index;colorspec ST` - Set ANSI color palette entry (index 0-15)
  - Color spec formats: `rgb:RR/GG/BB` or `#RRGGBB`
  - Example: `OSC 4;1;rgb:FF/00/00 ST` sets color 1 to red
- `OSC 104 ST` - Reset all ANSI colors to defaults
- `OSC 104;index ST` - Reset specific ANSI color to default

### Default Color Operations

- `OSC 10;? ST` - Query default foreground color → Response: `OSC 10;rgb:rrrr/gggg/bbbb ST`
- `OSC 10;colorspec ST` - Set default foreground color
- `OSC 110 ST` - Reset default foreground color

- `OSC 11;? ST` - Query default background color → Response: `OSC 11;rgb:rrrr/gggg/bbbb ST`
- `OSC 11;colorspec ST` - Set default background color
- `OSC 111 ST` - Reset default background color

- `OSC 12;? ST` - Query cursor color → Response: `OSC 12;rgb:rrrr/gggg/bbbb ST`
- `OSC 12;colorspec ST` - Set cursor color
- `OSC 112 ST` - Reset cursor color

### Notifications

#### iTerm2/ConEmu Style

`OSC 9;<message>ST`

Simple format with message only (no title). Send desktop-style notifications.

#### urxvt Style

`OSC 777;notify;<title>;<message>ST`

Structured notifications with both title and message. Use for desktop notifications, alerts, or completion notices.

**Security Note:** Notifications can be disabled using `disable_insecure_sequences` setting.

## DCS Sequences

Device Control String sequences for graphics and advanced features.

### Sixel Graphics

`DCS params q data ST`

Full VT340 Sixel graphics support for inline images.

- Parameters: Aspect ratio, background mode
- Data includes color definitions, raster attributes, and sixel data
- **Security Note:** Sixel graphics can be disabled using `disable_insecure_sequences` setting
- Configurable limits: max pixels, max colors, max graphics retained

**Example:**
```
DCS 0 ; 0 q
"1;1;100;100    # Raster attributes (100x100 pixels)
#0;2;100;100;100  # Define color 0 as RGB
#0 ????           # Draw sixel data with color 0
ST
```

See [Sixel Graphics Specification](https://vt100.net/docs/vt3xx-gp/chapter14.html) for details.

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
- [VT Feature Parity](VT_FEATURE_PARITY.md) - Detailed VT compatibility information
- [Advanced Features](ADVANCED_FEATURES.md) - Feature usage guides
- [xterm Control Sequences](https://invisible-island.net/xterm/ctlseqs/ctlseqs.html) - Official xterm documentation
