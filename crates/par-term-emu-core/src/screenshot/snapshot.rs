//! Lock-light screenshot rendering (ARC-004 remainder).
//!
//! `render_terminal` reads the grid view, cursor, theme values, and graphics
//! store. Under a shared `Arc<RwLock<Terminal>>` (`PtySession`) running the
//! full render — font shaping, pixel blitting, PNG/JPEG encoding — while
//! holding the read lock stalls the PTY reader's write lock for the whole
//! render. [`Terminal::render_snapshot`] copies that state (graphics pixel
//! buffers are `Arc`-shared), so a caller holds the lock only for the capture
//! and renders from the detached [`TerminalRenderSnapshot`].

use super::{render_grid, ScreenshotConfig, ScreenshotResult, SixelRenderMode};
use crate::cursor::Cursor;
use crate::graphics::{GraphicsStoreSnapshot, TerminalGraphic};
use crate::grid::Grid;
use crate::terminal::Terminal;
use std::path::Path;

/// Theme values the render path reads from the terminal, copied at capture
/// time so [`render_terminal_from_snapshot`] needs no terminal access.
#[derive(Debug, Clone, Copy)]
struct RenderTheme {
    link_color: (u8, u8, u8),
    bold_color: (u8, u8, u8),
    use_bold_color: bool,
    bold_brightening: bool,
    faint_text_alpha: f32,
    default_bg: (u8, u8, u8),
}

/// The terminal state a screenshot render reads, detached from the live
/// terminal. Captured with [`Terminal::render_snapshot`]; rendered with
/// [`render_terminal_from_snapshot`] / [`save_terminal_from_snapshot`].
#[derive(Debug, Clone)]
pub struct TerminalRenderSnapshot {
    /// The rendered view: visible screen, or the scrollback slice when the
    /// snapshot was taken with a `scrollback_offset` (see
    /// `Terminal::grid_with_scrollback`).
    grid: Grid,
    /// Cursor state, present only when the snapshot was taken at offset 0 —
    /// the same condition the live path used before rendering from it.
    cursor: Option<Cursor>,
    /// The graphics the render may draw. Empty when the snapshot was taken
    /// with a `scrollback_offset`: the live path renders no graphics there.
    graphics: GraphicsStoreSnapshot,
    theme: RenderTheme,
}

impl TerminalRenderSnapshot {
    /// The rendered view (visible screen or scrollback slice).
    pub fn grid(&self) -> &Grid {
        &self.grid
    }

    /// Cursor state; `None` for scrollback-offset snapshots.
    pub fn cursor(&self) -> Option<&Cursor> {
        self.cursor.as_ref()
    }

    /// The graphics captured alongside the view.
    pub fn graphics(&self) -> &GraphicsStoreSnapshot {
        &self.graphics
    }

    /// Apply the captured theme values to `config`: the same fills
    /// `render_terminal` made from the live terminal before this type
    /// existed.
    fn apply_theme(&self, config: &mut ScreenshotConfig) {
        if config.link_color.is_none() {
            config.link_color = Some(self.theme.link_color);
        }
        if config.bold_color.is_none() {
            config.bold_color = Some(self.theme.bold_color);
        }
        config.use_bold_color = self.theme.use_bold_color;
        config.bold_brightening = self.theme.bold_brightening;
        config.faint_text_alpha = self.theme.faint_text_alpha;
        if config.background_color.is_none() {
            config.background_color = Some(self.theme.default_bg);
        }
    }
}

impl Terminal {
    /// Copy the state a screenshot render reads: the scrollback-offset grid
    /// view, cursor, theme values, and graphics store. Graphics pixel buffers
    /// are `Arc`-shared, so the capture is a grid-cell clone plus reference
    /// bumps; hold the terminal lock for this call, then drop it and render
    /// from the returned snapshot.
    #[cfg(feature = "screenshot")]
    pub fn render_snapshot(&self, scrollback_offset: usize) -> TerminalRenderSnapshot {
        TerminalRenderSnapshot {
            grid: self.grid_with_scrollback(scrollback_offset),
            cursor: (scrollback_offset == 0).then_some(self.cursor),
            graphics: if scrollback_offset == 0 {
                self.graphics.graphics_store.snapshot()
            } else {
                GraphicsStoreSnapshot::default()
            },
            theme: RenderTheme {
                link_color: self.theme.link_color.to_rgb(),
                bold_color: self.theme.bold_color.to_rgb(),
                use_bold_color: self.theme.use_bold_color,
                bold_brightening: self.host.bold_brightening,
                faint_text_alpha: self.theme.faint_text_alpha,
                default_bg: self.theme.default_bg.to_rgb(),
            },
        }
    }
}

/// Render a terminal's visible buffer from a snapshot taken with
/// [`Terminal::render_snapshot`]. Byte-identical to
/// [`render_terminal`](super::render_terminal) over the same state.
pub fn render_terminal_from_snapshot(
    snap: &TerminalRenderSnapshot,
    config: ScreenshotConfig,
) -> ScreenshotResult<Vec<u8>> {
    let mut config = config;
    snap.apply_theme(&mut config);

    let cursor = if config.render_cursor {
        snap.cursor.as_ref()
    } else {
        None
    };
    let graphics: &[TerminalGraphic] = if config.sixel_render_mode == SixelRenderMode::Disabled {
        &[]
    } else {
        snap.graphics.placements()
    };
    render_grid(&snap.grid, cursor, graphics, config)
}

/// Render from a snapshot and save the image to `path`; the snapshot-side
/// counterpart of [`save_terminal`](super::save_terminal).
pub fn save_terminal_from_snapshot(
    snap: &TerminalRenderSnapshot,
    path: &Path,
    config: ScreenshotConfig,
) -> ScreenshotResult<()> {
    let bytes = render_terminal_from_snapshot(snap, config)?;
    std::fs::write(path, bytes)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::color::Color;
    use crate::graphics::{next_graphic_id, GraphicProtocol};
    use crate::screenshot::render_terminal;
    use std::sync::Arc;

    /// A terminal with scrollback, themed colors, and two on-screen graphics
    /// (Sixel + Kitty), so a parity render exercises text, theme, cursor, and
    /// both graphics-blit paths.
    fn busy_terminal() -> Terminal {
        let mut term = Terminal::with_scrollback(40, 12, 100);
        for i in 0..30 {
            term.process(format!("scroll line {i}\r\n").as_bytes());
        }
        term.process(b"\x1b[1;31mbold red\x1b[0m and \x1b[4mlink text\x1b[0m plain");
        term.set_link_color(Color::Rgb(10, 20, 30));
        term.set_bold_color(Color::Rgb(200, 100, 50));
        term.set_use_bold_color(true);
        term.set_faint_text_alpha(0.35);

        let mut sixel = TerminalGraphic::new(
            next_graphic_id(),
            GraphicProtocol::Sixel,
            (2, 3),
            16,
            8,
            gradient_pixels(16, 8, 1),
        );
        sixel.set_cell_dimensions(8, 16);
        term.graphics_store_mut().add_graphic(sixel);

        term.graphics_store_mut()
            .add_graphic(TerminalGraphic::with_shared_pixels(
                next_graphic_id(),
                GraphicProtocol::Kitty,
                (24, 6),
                12,
                10,
                Arc::new(gradient_pixels(12, 10, 2)),
            ));
        term
    }

    /// A deterministic RGBA buffer with per-position variation, so a missing
    /// or misplaced graphic blit changes the rendered bytes.
    fn gradient_pixels(width: usize, height: usize, salt: u8) -> Vec<u8> {
        (0..width * height * 4)
            .map(|i| ((i * 7 + salt as usize * 31) % 251) as u8)
            .collect()
    }

    /// The pre-snapshot body of `render_terminal`, kept as the parity
    /// reference: it reads the live terminal directly instead of a snapshot.
    fn live_reference_render(
        term: &Terminal,
        mut config: ScreenshotConfig,
        scrollback_offset: usize,
    ) -> ScreenshotResult<Vec<u8>> {
        if config.link_color.is_none() {
            config.link_color = Some(term.theme.link_color.to_rgb());
        }
        if config.bold_color.is_none() {
            config.bold_color = Some(term.theme.bold_color.to_rgb());
        }
        config.use_bold_color = term.theme.use_bold_color;
        config.bold_brightening = term.host.bold_brightening;
        config.faint_text_alpha = term.theme.faint_text_alpha;
        if config.background_color.is_none() {
            config.background_color = Some(term.theme.default_bg.to_rgb());
        }

        let grid = term.grid_with_scrollback(scrollback_offset);
        let cursor = if config.render_cursor && scrollback_offset == 0 {
            Some(&term.cursor)
        } else {
            None
        };
        let graphics =
            if config.sixel_render_mode != SixelRenderMode::Disabled && scrollback_offset == 0 {
                term.all_graphics()
            } else {
                &[]
            };
        render_grid(&grid, cursor, graphics, config)
    }

    #[test]
    fn snapshot_render_matches_live_render_with_graphics() {
        let term = busy_terminal();
        for mode in [
            SixelRenderMode::HalfBlocks,
            SixelRenderMode::Pixels,
            SixelRenderMode::Disabled,
        ] {
            let config = ScreenshotConfig {
                sixel_render_mode: mode,
                ..ScreenshotConfig::default()
            };
            let from_snapshot =
                render_terminal_from_snapshot(&term.render_snapshot(0), config.clone());
            let from_live = live_reference_render(&term, config, 0);
            assert_eq!(
                from_snapshot.unwrap(),
                from_live.unwrap(),
                "snapshot render must be byte-identical to the live render (mode {mode:?})"
            );
        }
    }

    #[test]
    fn snapshot_render_matches_live_render_with_cursor_and_scrollback_offset() {
        let term = busy_terminal();
        let config = ScreenshotConfig {
            render_cursor: true,
            ..ScreenshotConfig::default()
        };
        let from_snapshot = render_terminal_from_snapshot(&term.render_snapshot(0), config.clone());
        let from_live = live_reference_render(&term, config, 0);
        assert_eq!(from_snapshot.unwrap(), from_live.unwrap());

        // A scrollback-offset snapshot drops cursor and graphics on both
        // paths alike.
        let offset_config = ScreenshotConfig::default();
        let from_snapshot =
            render_terminal_from_snapshot(&term.render_snapshot(2), offset_config.clone());
        let from_live = live_reference_render(&term, offset_config, 2);
        assert_eq!(from_snapshot.unwrap(), from_live.unwrap());
    }

    #[test]
    fn offset_snapshot_carries_no_cursor_or_graphics() {
        let term = busy_terminal();
        let snap = term.render_snapshot(2);
        assert!(snap.cursor().is_none());
        assert!(snap.graphics().is_empty());
        assert_eq!(snap.grid().rows(), 12);
        assert_eq!(snap.grid().cols(), 40);

        let at_zero = term.render_snapshot(0);
        assert!(at_zero.cursor().is_some());
        assert_eq!(at_zero.graphics().placements().len(), 2);
    }

    #[test]
    fn snapshot_render_is_detached_from_later_terminal_activity() {
        let mut term = busy_terminal();
        let config = ScreenshotConfig::default();
        let snap = term.render_snapshot(0);
        let before = render_terminal_from_snapshot(&snap, config.clone()).unwrap();

        // Mutate the terminal on both channels the render reads: more text
        // and one more on-screen graphic.
        term.process(b"\r\nlater output");
        term.graphics_store_mut().add_graphic(TerminalGraphic::new(
            next_graphic_id(),
            GraphicProtocol::Sixel,
            (10, 8),
            8,
            8,
            gradient_pixels(8, 8, 9),
        ));

        let after = render_terminal_from_snapshot(&snap, config.clone()).unwrap();
        assert_eq!(before, after, "snapshot must not observe later output");
        let live_now = render_terminal(&term, config, 0).unwrap();
        assert_ne!(
            live_now, after,
            "the later output must be visible to a fresh live render"
        );
    }

    /// Save-from-snapshot writes a readable file (round-trip through
    /// `save_terminal_from_snapshot`).
    #[test]
    fn save_from_snapshot_writes_the_rendered_bytes() {
        let term = busy_terminal();
        let snap = term.render_snapshot(0);
        let path = std::env::temp_dir().join(format!(
            "ptecr-snap-test-{}-{}.png",
            std::process::id(),
            crate::text_utils::unix_millis()
        ));
        let result = save_terminal_from_snapshot(&snap, &path, ScreenshotConfig::default());
        let saved = result.map(|()| std::fs::read(&path).unwrap_or_default());
        let _ = std::fs::remove_file(&path);
        let saved = saved.unwrap();
        assert_eq!(&saved[0..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(
            saved,
            render_terminal_from_snapshot(&snap, ScreenshotConfig::default()).unwrap()
        );
    }
}
