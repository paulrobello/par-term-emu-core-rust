//! Kitty graphics on the host terminal (config `kitty-graphics`, herdr's
//! `terminal.kitty_graphics`, default on).
//!
//! Each pane's emulator already parses the pane's kitty APC sequences
//! into RGBA placements; ratatui cells cannot carry them, so the render
//! client re-emits every visible placement to the host as its OWN kitty
//! commands: one upload per distinct image (`a=t`, RGBA `f=32`, direct
//! base64 chunks), then a placement (`a=p`) at the pane cell the image
//! occupies on screen, cropped to the pane's content view. Placements
//! carry `C=1` (the host cursor never moves, so a placement on the
//! bottom row cannot scroll the frame) and `q=2` (the host never
//! answers into the client's stdin). Uploads and placements diff
//! against what the host already holds, so a quiet frame writes nothing.
//!
//! Fallback: the launch probe asks the host with a kitty query
//! (`a=q`, followed by DA1 as the answer-ordering sentinel — see
//! `conn::KITTY_QUERY`). Nothing is ever drawn unless the host answered
//! `OK` — a host without kitty graphics receives only the one query,
//! which the seed's screen clear wipes if the host printed it.

use super::{FlushSink, PaneRect, PaneRenderer, WindowSession};
use base64::Engine as _;
use par_term_emu_core::graphics::{GraphicProtocol, TerminalGraphic};
use std::collections::{HashMap, HashSet};
use std::fmt::Write as _;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

/// Base64 bytes per upload chunk (the kitty spec's 4096 ceiling).
const CHUNK: usize = 4096;

/// Host image ids start here; placements number from 1.
const HOST_ID_BASE: u32 = 10_000;

/// One distinct image's identity: the shared pixel buffer, its size,
/// and a sampled content fingerprint (a buffer re-allocated at the same
/// address with other pixels must not reuse the host's copy).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
struct ImageSig {
    ptr: usize,
    width: usize,
    height: usize,
    fingerprint: u64,
}

impl ImageSig {
    fn of(graphic: &TerminalGraphic) -> Self {
        let pixels = graphic.pixels.as_slice();
        let mut hasher = std::collections::hash_map::DefaultHasher::new();
        pixels.len().hash(&mut hasher);
        let stride = (pixels.len() / 64).max(1);
        for byte in pixels.iter().step_by(stride) {
            byte.hash(&mut hasher);
        }
        Self {
            ptr: Arc::as_ptr(&graphic.pixels) as usize,
            width: graphic.width,
            height: graphic.height,
            fingerprint: hasher.finish(),
        }
    }
}

/// Where and how one placement shows: the frame cell of its top-left,
/// its cell extent (always stated, so the host scales the image into
/// exactly the cells the pane's grid reserved for it), the source crop
/// in image pixels, the in-cell pixel offset, and z.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Geom {
    pub x: u16,
    pub y: u16,
    pub cols: u32,
    pub rows: u32,
    pub src_x: u32,
    pub src_y: u32,
    pub src_w: u32,
    pub src_h: u32,
    pub x_off: u32,
    pub y_off: u32,
    pub z: i32,
}

/// One placement a frame wants on the host.
pub(crate) struct Placement {
    /// `(pane, graphic id)`: the placement's identity across frames.
    key: (u32, u64),
    sig: ImageSig,
    width: usize,
    height: usize,
    pixels: Arc<Vec<u8>>,
    pub(crate) geom: Geom,
}

/// The cropped geometry of `graphic` inside a content view of
/// `max_cols` x `max_rows` cells scrolled `scroll` rows into history,
/// at `cell` pixels per cell. `None` when nothing of it is visible.
pub(crate) fn clip(
    graphic: &TerminalGraphic,
    scroll: usize,
    max_cols: u16,
    max_rows: u16,
    cell: (u32, u32),
) -> Option<Geom> {
    let (span_cols, span_rows) = graphic.cell_span(cell.0, cell.1);
    let (span_cols, span_rows) = (span_cols as u32, span_rows as u32);
    let top_clip = graphic.scroll_offset_rows as u32;
    if span_cols == 0 || span_rows <= top_clip {
        return None;
    }
    let row0 = graphic.position.1.checked_add(scroll)?;
    let col0 = graphic.position.0;
    if row0 >= usize::from(max_rows) || col0 >= usize::from(max_cols) {
        return None;
    }
    let rows = (span_rows - top_clip).min(u32::from(max_rows) - row0 as u32);
    let cols = span_cols.min(u32::from(max_cols) - col0 as u32);

    let p = &graphic.placement;
    let (img_w, img_h) = (graphic.width as u32, graphic.height as u32);
    let src_x = p.source_x.min(img_w);
    let src_y = p.source_y.min(img_h);
    let full_w = img_w - src_x;
    let full_h = img_h - src_y;
    let src_w = if p.source_width > 0 {
        p.source_width.min(full_w)
    } else {
        full_w
    };
    let src_h = if p.source_height > 0 {
        p.source_height.min(full_h)
    } else {
        full_h
    };
    if src_w == 0 || src_h == 0 {
        return None;
    }
    let cropped = top_clip > 0 || rows < span_rows || cols < span_cols;
    let (src_y, src_w, src_h) = if cropped {
        let scale =
            |v: u32, num: u32, den: u32| (u64::from(v) * u64::from(num) / u64::from(den)) as u32;
        (
            src_y + scale(src_h, top_clip, span_rows),
            scale(src_w, cols, span_cols).max(1),
            scale(src_h, rows, span_rows).max(1),
        )
    } else {
        (src_y, src_w, src_h)
    };
    Some(Geom {
        x: col0 as u16,
        y: row0 as u16,
        cols,
        rows,
        src_x,
        src_y,
        src_w,
        src_h,
        x_off: p.x_offset,
        y_off: if top_clip > 0 { 0 } else { p.y_offset },
        z: p.z_index,
    })
}

impl PaneRenderer {
    /// The cell pixel size this client reported to the daemon (the
    /// probe's XTWINOPS answer): every emulator re-derives image spans
    /// from it.
    pub(crate) fn set_cell_px(&mut self, cell_px: (u32, u32)) {
        if self.cell_px == cell_px {
            return;
        }
        self.cell_px = cell_px;
        for emulator in self.emulators.values_mut() {
            emulator.set_cell_px(cell_px);
        }
        self.dirty = true;
    }

    /// Feed a pane's `refresh-client -t` replay. The replay rebuilds the
    /// grid from the daemon's screen state, which carries no images, so
    /// the graphics the mirror held are dropped first — kept, they would
    /// sit at their pre-replay rows over the rebuilt (scrolled, reflowed)
    /// text.
    pub(crate) fn feed_replay(&mut self, pane: u32, bytes: &[u8]) {
        if let Some(emulator) = self.emulators.get_mut(&pane) {
            emulator.clear_graphics();
        }
        self.feed_output(pane, bytes);
    }

    /// Every kitty placement visible in the current layout, in frame
    /// coordinates — the same rect, inset, and clamps
    /// [`PaneRenderer::paint_pane`] maps cells through. Placements the
    /// toast card overlaps are left out (an image would paint over it).
    pub(crate) fn kitty_placements(&self) -> Vec<Placement> {
        let toast = self.toast_rect();
        let mut out = Vec::new();
        for rect in &self.layout {
            self.pane_kitty_placements(rect, toast, &mut out);
        }
        out
    }

    fn pane_kitty_placements(
        &self,
        rect: &PaneRect,
        toast: Option<(u16, u16, u16, u16)>,
        out: &mut Vec<Placement>,
    ) {
        let Some(emulator) = self.emulators.get(&rect.pane) else {
            return;
        };
        let term = emulator.terminal();
        let graphics = term.all_graphics();
        if graphics.is_empty() {
            return;
        }
        let (inset_x, inset_y, view_w, view_h) = self.content_view(rect);
        let grid = term.active_grid();
        let origin_x = rect.x + self.geometry.sidebar_w + inset_x;
        let origin_y = rect.y + inset_y;
        let max_rows = view_h
            .min(grid.rows() as u16)
            .min(self.height.saturating_sub(origin_y));
        let max_cols = view_w
            .min(grid.cols() as u16)
            .min(self.width.saturating_sub(origin_x));
        for graphic in graphics {
            if graphic.protocol != GraphicProtocol::Kitty
                || graphic.is_virtual
                || graphic.pixels.len() != graphic.width * graphic.height * 4
            {
                continue;
            }
            let Some(mut geom) = clip(
                graphic,
                emulator.scroll_offset(),
                max_cols,
                max_rows,
                self.cell_px,
            ) else {
                continue;
            };
            geom.x += origin_x;
            geom.y += origin_y;
            if let Some((tx, ty, tw, th)) = toast {
                let overlaps = u32::from(geom.x) < u32::from(tx) + u32::from(tw)
                    && u32::from(geom.x) + geom.cols > u32::from(tx)
                    && u32::from(geom.y) < u32::from(ty) + u32::from(th)
                    && u32::from(geom.y) + geom.rows > u32::from(ty);
                if overlaps {
                    continue;
                }
            }
            out.push(Placement {
                key: (rect.pane, graphic.id),
                sig: ImageSig::of(graphic),
                width: graphic.width,
                height: graphic.height,
                pixels: Arc::clone(&graphic.pixels),
                geom,
            });
        }
    }
}

impl WindowSession {
    /// The host's answer to the kitty query (the launch probe, or a late
    /// reply the stdin parser caught): a supported host draws from the
    /// next frame on.
    pub(super) fn set_kitty_supported(&mut self, ok: bool) {
        if self.kitty.supported != ok {
            self.kitty.supported = ok;
            self.renderer.mark_all_dirty();
        }
    }

    /// The config's `kitty-graphics` (launch and reload). Turning it off
    /// takes everything already drawn off the host on the next frame.
    pub(super) fn set_kitty_enabled(&mut self, on: bool) {
        if self.kitty.enabled != on {
            self.kitty.enabled = on;
            self.renderer.mark_all_dirty();
        }
    }

    /// Bring the host's kitty graphics in line with the frame just
    /// flushed. Returns whether bytes went out (their CUPs move the host
    /// cursor, so the caller must re-place it). Placements hide while a
    /// modal or popup covers the panes — z-ordered images would paint
    /// over them — and come back when it closes.
    pub(super) fn frame_graphics(&mut self, sink: &mut dyn FlushSink, covered: bool) -> bool {
        let bytes = if !self.kitty.active() {
            if !self.kitty.holds_any() {
                return false;
            }
            self.kitty.clear()
        } else if covered {
            self.kitty.hide_all()
        } else {
            let desired = self.renderer.kitty_placements();
            let geometry = self.geometry;
            self.kitty
                .update(&desired, |x, y| geometry.frame_to_host(x, y))
        };
        if bytes.is_empty() {
            return false;
        }
        sink.write_raw(&bytes);
        true
    }
}

/// What the host terminal holds of the client's kitty graphics, and the
/// gates: `enabled` (config `kitty-graphics`) and `supported` (the host
/// answered the kitty query). Both must hold before anything is drawn.
#[derive(Debug)]
pub(crate) struct HostGraphics {
    pub enabled: bool,
    pub supported: bool,
    /// Host image id per distinct image.
    ids: HashMap<ImageSig, u32>,
    /// Host image ids whose pixels the host holds.
    uploaded: HashSet<u32>,
    /// Host placement id per `(pane, graphic id)`.
    pids: HashMap<(u32, u64), u32>,
    /// What each placement shows on the host now: image id, placement
    /// id, host-cell geometry.
    placed: HashMap<(u32, u64), (u32, u32, Geom)>,
    next_id: u32,
    next_pid: u32,
}

impl Default for HostGraphics {
    fn default() -> Self {
        Self {
            enabled: true,
            supported: false,
            ids: HashMap::new(),
            uploaded: HashSet::new(),
            pids: HashMap::new(),
            placed: HashMap::new(),
            next_id: HOST_ID_BASE,
            next_pid: 1,
        }
    }
}

impl HostGraphics {
    /// Both gates hold: frames draw graphics.
    pub(crate) fn active(&self) -> bool {
        self.enabled && self.supported
    }

    /// Whether anything of the client's is on the host.
    pub(crate) fn holds_any(&self) -> bool {
        !self.ids.is_empty() || !self.placed.is_empty()
    }

    /// The host screen is about to be cleared and repainted
    /// (`repaint_all`): the returned bytes delete every placement and
    /// image the client put there — hosts differ on whether a screen
    /// clear removes kitty state, and a refit's replay may drop an image
    /// the next frame would otherwise never free — and the next frame
    /// re-uploads and re-places what is still wanted.
    pub(crate) fn invalidate(&mut self) -> Vec<u8> {
        let mut out = self.hide_all();
        out.extend(self.clear());
        out
    }

    /// Take every placement off the host, keeping the uploaded images
    /// (a modal or popup covers the panes; it closes soon).
    pub(crate) fn hide_all(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        let mut keys: Vec<_> = self.placed.drain().map(|(_, v)| v).collect();
        keys.sort_unstable_by_key(|(id, pid, _)| (*id, *pid));
        for (id, pid, _) in keys {
            delete_placement(&mut out, id, pid);
        }
        out
    }

    /// Delete everything the client put on the host (detach, or the
    /// option turned off).
    pub(crate) fn clear(&mut self) -> Vec<u8> {
        let mut out = Vec::new();
        let mut ids: Vec<u32> = self.ids.drain().map(|(_, id)| id).collect();
        ids.sort_unstable();
        for id in ids {
            delete_image(&mut out, id);
        }
        self.uploaded.clear();
        self.placed.clear();
        self.pids.clear();
        out
    }

    /// The bytes that bring the host from what it holds to `desired`,
    /// whose geometry is in frame cells (`to_host` rebases them):
    /// placements that left are deleted, images nothing shows any more
    /// are freed, new images upload, and new or moved placements place.
    /// Empty when the host already matches.
    pub(crate) fn update(
        &mut self,
        desired: &[Placement],
        to_host: impl Fn(u16, u16) -> (u16, u16),
    ) -> Vec<u8> {
        let mut out = Vec::new();
        let want_sigs: HashSet<ImageSig> = desired.iter().map(|p| p.sig).collect();
        let want: HashMap<(u32, u64), u32> = desired
            .iter()
            .map(|p| (p.key, self.ids.get(&p.sig).copied().unwrap_or(0)))
            .collect();

        let mut stale: Vec<((u32, u64), u32, u32)> = self
            .placed
            .iter()
            .filter(|(key, (id, _, _))| want.get(*key) != Some(id))
            .map(|(key, (id, pid, _))| (*key, *id, *pid))
            .collect();
        stale.sort_unstable_by_key(|(key, _, _)| *key);
        for (key, id, pid) in stale {
            delete_placement(&mut out, id, pid);
            self.placed.remove(&key);
        }
        let mut freed: Vec<(ImageSig, u32)> = self
            .ids
            .iter()
            .filter(|(sig, _)| !want_sigs.contains(*sig))
            .map(|(sig, id)| (*sig, *id))
            .collect();
        freed.sort_unstable_by_key(|(_, id)| *id);
        for (sig, id) in freed {
            self.uploaded.remove(&id);
            delete_image(&mut out, id);
            self.ids.remove(&sig);
        }
        self.pids.retain(|key, _| want.contains_key(key));

        for p in desired {
            let id = match self.ids.get(&p.sig) {
                Some(id) => *id,
                None => {
                    let id = self.next_id;
                    self.next_id = self.next_id.wrapping_add(1).max(HOST_ID_BASE);
                    self.ids.insert(p.sig, id);
                    id
                }
            };
            if self.uploaded.insert(id) {
                upload(&mut out, id, p.width, p.height, &p.pixels);
            }
            let pid = *self.pids.entry(p.key).or_insert_with(|| {
                let pid = self.next_pid;
                self.next_pid = self.next_pid.wrapping_add(1).max(1);
                pid
            });
            let (hx, hy) = to_host(p.geom.x, p.geom.y);
            let geom = Geom {
                x: hx,
                y: hy,
                ..p.geom
            };
            if self.placed.get(&p.key) != Some(&(id, pid, geom)) {
                place(&mut out, id, pid, &geom);
                self.placed.insert(p.key, (id, pid, geom));
            }
        }
        out
    }
}

/// `a=t` upload of RGBA pixels in base64 chunks: the first chunk carries
/// the keys, the rest only `m=` (and `q`).
fn upload(out: &mut Vec<u8>, id: u32, width: usize, height: usize, pixels: &[u8]) {
    let data = base64::engine::general_purpose::STANDARD.encode(pixels);
    let chunks: Vec<&[u8]> = data.as_bytes().chunks(CHUNK).collect();
    let last = chunks.len().saturating_sub(1);
    for (i, chunk) in chunks.iter().enumerate() {
        let more = u8::from(i < last);
        let mut head = String::new();
        if i == 0 {
            let _ = write!(
                head,
                "a=t,t=d,f=32,s={width},v={height},i={id},q=2,m={more}"
            );
        } else {
            let _ = write!(head, "q=2,m={more}");
        }
        out.extend_from_slice(b"\x1b_G");
        out.extend_from_slice(head.as_bytes());
        out.push(b';');
        out.extend_from_slice(chunk);
        out.extend_from_slice(b"\x1b\\");
    }
}

/// CUP to the placement's host cell, then `a=p` with `C=1` (no cursor
/// move) and the crop/extent/offset keys that differ from the defaults.
fn place(out: &mut Vec<u8>, id: u32, pid: u32, g: &Geom) {
    let mut s = format!(
        "\x1b[{};{}H\x1b_Ga=p,i={id},p={pid},c={},r={},C=1,q=2",
        g.y + 1,
        g.x + 1,
        g.cols,
        g.rows
    );
    if g.src_x > 0 {
        let _ = write!(s, ",x={}", g.src_x);
    }
    if g.src_y > 0 {
        let _ = write!(s, ",y={}", g.src_y);
    }
    let _ = write!(s, ",w={},h={}", g.src_w, g.src_h);
    if g.x_off > 0 {
        let _ = write!(s, ",X={}", g.x_off);
    }
    if g.y_off > 0 {
        let _ = write!(s, ",Y={}", g.y_off);
    }
    if g.z != 0 {
        let _ = write!(s, ",z={}", g.z);
    }
    s.push_str(";\x1b\\");
    out.extend_from_slice(s.as_bytes());
}

fn delete_placement(out: &mut Vec<u8>, id: u32, pid: u32) {
    let _ = write!(StringSink(out), "\x1b_Ga=d,d=i,i={id},p={pid},q=2;\x1b\\");
}

fn delete_image(out: &mut Vec<u8>, id: u32) {
    let _ = write!(StringSink(out), "\x1b_Ga=d,d=I,i={id},q=2;\x1b\\");
}

/// `fmt::Write` over a byte buffer (the escapes are ASCII).
struct StringSink<'a>(&'a mut Vec<u8>);

impl std::fmt::Write for StringSink<'_> {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        self.0.extend_from_slice(s.as_bytes());
        Ok(())
    }
}
