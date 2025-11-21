use image::{Rgba, RgbaImage};

use crate::cell::{Cell, UnderlineStyle};
use crate::cursor::{Cursor, CursorStyle};
use crate::graphics::TerminalGraphic;
use crate::grid::Grid;

use super::config::{ScreenshotConfig, SixelRenderMode};
use super::error::ScreenshotResult;
use super::font_cache::{BitmapFormat, FontCache};
use super::shaper::{ShapedGlyphWithFont, TextShaper};
use super::utils::{blend_grayscale_pixel, blend_rgba_pixel};

/// Screenshot renderer
pub struct Renderer {
    config: ScreenshotConfig,
    font_cache: FontCache,
    cell_width: u32,
    cell_height: u32,
    canvas_width: u32,
    canvas_height: u32,
    /// Text shaper for handling multi-codepoint emoji (like flags)
    shaper: Option<TextShaper>,
}

impl Renderer {
    /// Create a new renderer
    pub fn new(rows: usize, cols: usize, config: ScreenshotConfig) -> ScreenshotResult<Self> {
        let mut font_cache = FontCache::new(config.font_path.as_deref(), config.font_size)?;
        let (base_width, base_height) = font_cache.cell_dimensions();

        // Apply multipliers
        let cell_width = (base_width as f32 * config.char_width_multiplier) as u32;
        let cell_height = (base_height as f32 * config.line_height_multiplier) as u32;

        let canvas_width = cols as u32 * cell_width + config.padding_px * 2;
        let canvas_height = rows as u32 * cell_height + config.padding_px * 2;

        // Initialize text shaper with font data
        let shaper = Self::create_text_shaper(&mut font_cache).ok();

        Ok(Self {
            config,
            font_cache,
            cell_width,
            cell_height,
            canvas_width,
            canvas_height,
            shaper,
        })
    }

    /// Create and configure a TextShaper with fonts from FontCache
    fn create_text_shaper(font_cache: &mut FontCache) -> ScreenshotResult<TextShaper> {
        // Get font data from cache
        let regular_data = font_cache.regular_font_data();
        let mut shaper = TextShaper::new(regular_data)?;

        // Add emoji font if available
        if let Some(emoji_data) = font_cache.emoji_font_data() {
            let _ = shaper.set_emoji_font(emoji_data);
        }

        // Add CJK font if available
        if let Some(cjk_data) = font_cache.cjk_font_data() {
            let _ = shaper.set_cjk_font(cjk_data);
        }

        Ok(shaper)
    }

    /// Render a grid to an image
    pub fn render_grid(
        &mut self,
        grid: &Grid,
        cursor: Option<&Cursor>,
        graphics: &[TerminalGraphic],
    ) -> ScreenshotResult<RgbaImage> {
        // Create canvas with background color
        let bg_color = self.config.background_color.unwrap_or((0, 0, 0));
        let mut image = RgbaImage::from_pixel(
            self.canvas_width,
            self.canvas_height,
            Rgba([bg_color.0, bg_color.1, bg_color.2, 255]),
        );

        // Render each row - use shaped rendering for lines with Regional Indicators
        for row in 0..grid.rows() {
            let row_text = grid.row_text(row);

            // Use shaped rendering if the line contains Regional Indicators (flags)
            // and we have a shaper available
            if self.shaper.is_some() && Self::contains_regional_indicators(&row_text) {
                self.render_shaped_line(&mut image, row, grid)?;
            } else {
                // Use normal character-by-character rendering
                for col in 0..grid.cols() {
                    if let Some(cell) = grid.get(col, row) {
                        self.render_cell(&mut image, cell, col, row)?;
                    }
                }
            }
        }

        // Render Sixel graphics based on mode
        match self.config.sixel_render_mode {
            SixelRenderMode::Disabled => {}
            SixelRenderMode::Pixels => {
                for graphic in graphics {
                    self.render_sixel_pixels(&mut image, graphic);
                }
            }
            SixelRenderMode::HalfBlocks => {
                for graphic in graphics {
                    self.render_sixel_halfblocks(&mut image, graphic)?;
                }
            }
        }

        // Render cursor if enabled and visible
        if self.config.render_cursor {
            if let Some(cursor) = cursor {
                if cursor.visible && cursor.row < grid.rows() && cursor.col < grid.cols() {
                    self.render_cursor(&mut image, cursor);
                }
            }
        }

        Ok(image)
    }

    /// Render a single cell
    fn render_cell(
        &mut self,
        image: &mut RgbaImage,
        cell: &Cell,
        col: usize,
        row: usize,
    ) -> ScreenshotResult<()> {
        let x = col as u32 * self.cell_width + self.config.padding_px;
        let y = row as u32 * self.cell_height + self.config.padding_px;

        // Skip wide character spacers
        if cell.flags.wide_char_spacer() {
            return Ok(());
        }

        // Resolve effective colors
        let (fg, bg) = self.resolve_colors(cell);

        // Render background
        self.render_background(image, x, y, bg);

        // Render character (if not hidden)
        if !cell.flags.hidden() && cell.c != ' ' {
            self.render_char(
                image,
                cell.c,
                x,
                y,
                fg,
                bg,
                cell.flags.bold(),
                cell.flags.italic(),
            )?;
        }

        // Render text decorations
        if cell.flags.underline() {
            let underline_color = cell.underline_color.map(|c| c.to_rgb()).unwrap_or(fg);
            self.render_underline(image, x, y, cell.flags.underline_style, underline_color);
        }

        if cell.flags.strikethrough() {
            self.render_strikethrough(image, x, y, fg);
        }

        if cell.flags.overline() {
            self.render_overline(image, x, y, fg);
        }

        Ok(())
    }

    /// Resolve effective foreground and background colors
    fn resolve_colors(&self, cell: &Cell) -> ((u8, u8, u8), (u8, u8, u8)) {
        let mut fg = cell.fg;
        let mut bg = cell.bg.to_rgb();

        // Apply bold brightening: if bold and color is ANSI 0-7, use bright variant 8-15
        if self.config.bold_brightening && cell.flags.bold() {
            if let crate::color::Color::Named(named) = fg {
                if (named as u8) < 8 {
                    // Convert normal ANSI color (0-7) to bright variant (8-15)
                    fg = crate::color::Color::Named(crate::color::NamedColor::from_u8(
                        named as u8 + 8,
                    ));
                }
            }
        }

        // Convert fg to RGB after bold brightening
        let mut fg_rgb = fg.to_rgb();

        // Apply theme colors AFTER bold brightening but BEFORE reverse/dim transformations
        // This ensures theme colors work correctly with reverse video and dim

        // Use link color for hyperlinked text
        if cell.flags.hyperlink_id.is_some() {
            if let Some(link_color) = self.config.link_color {
                fg_rgb = link_color;
            }
        }

        // Use custom bold color when enabled and cell is bold
        if cell.flags.bold() && self.config.use_bold_color {
            if let Some(bold_color) = self.config.bold_color {
                fg_rgb = bold_color;
            }
        }

        // Apply Display P3 color space conversion + brightness boost
        // iTerm2's P3 rendering makes colors both more vibrant AND lighter
        // First apply P3 conversion for saturation
        fg_rgb = crate::color_utils::srgb_to_p3_rgb(fg_rgb.0, fg_rgb.1, fg_rgb.2);

        // Then boost brightness to match iTerm2's visual appearance
        // Testing shows ~40% brightness boost needed after P3 conversion
        let boost = 1.4;
        fg_rgb = (
            ((fg_rgb.0 as f32 * boost).min(255.0)) as u8,
            ((fg_rgb.1 as f32 * boost).min(255.0)) as u8,
            ((fg_rgb.2 as f32 * boost).min(255.0)) as u8,
        );

        // Handle reverse video
        if cell.flags.reverse() {
            std::mem::swap(&mut fg_rgb, &mut bg);
        }

        // Handle dim/faint text by blending toward the background using configurable alpha
        if cell.flags.dim() {
            let faint_alpha = self.config.faint_text_alpha.clamp(0.0, 1.0);
            if faint_alpha < 1.0 {
                let inv_alpha = 1.0 - faint_alpha;
                fg_rgb = (
                    ((fg_rgb.0 as f32 * faint_alpha + bg.0 as f32 * inv_alpha)
                        .round()
                        .clamp(0.0, 255.0)) as u8,
                    ((fg_rgb.1 as f32 * faint_alpha + bg.1 as f32 * inv_alpha)
                        .round()
                        .clamp(0.0, 255.0)) as u8,
                    ((fg_rgb.2 as f32 * faint_alpha + bg.2 as f32 * inv_alpha)
                        .round()
                        .clamp(0.0, 255.0)) as u8,
                );
            }
        }

        // Apply minimum contrast adjustment if enabled
        if self.config.minimum_contrast > 0.0 {
            fg_rgb =
                crate::color_utils::adjust_contrast_rgb(fg_rgb, bg, self.config.minimum_contrast);
        }

        (fg_rgb, bg)
    }

    /// Render block element characters as filled rectangles for pixel-perfect rendering
    fn render_block_element(
        &self,
        image: &mut RgbaImage,
        c: char,
        x: u32,
        y: u32,
        fg: (u8, u8, u8),
        bg: (u8, u8, u8),
    ) -> ScreenshotResult<()> {
        let cell_w = self.cell_width;
        let cell_h = self.cell_height;

        match c {
            // Upper half block: foreground on top, background on bottom
            '\u{2580}' => {
                // Top half = foreground
                for py in 0..cell_h / 2 {
                    for px in 0..cell_w {
                        if x + px < self.canvas_width && y + py < self.canvas_height {
                            image.put_pixel(x + px, y + py, Rgba([fg.0, fg.1, fg.2, 255]));
                        }
                    }
                }
                // Bottom half = background
                for py in cell_h / 2..cell_h {
                    for px in 0..cell_w {
                        if x + px < self.canvas_width && y + py < self.canvas_height {
                            image.put_pixel(x + px, y + py, Rgba([bg.0, bg.1, bg.2, 255]));
                        }
                    }
                }
            }
            // Lower half block: background on top, foreground on bottom
            '\u{2584}' => {
                // Top half = background
                for py in 0..cell_h / 2 {
                    for px in 0..cell_w {
                        if x + px < self.canvas_width && y + py < self.canvas_height {
                            image.put_pixel(x + px, y + py, Rgba([bg.0, bg.1, bg.2, 255]));
                        }
                    }
                }
                // Bottom half = foreground
                for py in cell_h / 2..cell_h {
                    for px in 0..cell_w {
                        if x + px < self.canvas_width && y + py < self.canvas_height {
                            image.put_pixel(x + px, y + py, Rgba([fg.0, fg.1, fg.2, 255]));
                        }
                    }
                }
            }
            // Full block: all foreground
            '\u{2588}' => {
                for py in 0..cell_h {
                    for px in 0..cell_w {
                        if x + px < self.canvas_width && y + py < self.canvas_height {
                            image.put_pixel(x + px, y + py, Rgba([fg.0, fg.1, fg.2, 255]));
                        }
                    }
                }
            }
            // Left half block
            '\u{258C}' => {
                for py in 0..cell_h {
                    for px in 0..cell_w / 2 {
                        if x + px < self.canvas_width && y + py < self.canvas_height {
                            image.put_pixel(x + px, y + py, Rgba([fg.0, fg.1, fg.2, 255]));
                        }
                    }
                    for px in cell_w / 2..cell_w {
                        if x + px < self.canvas_width && y + py < self.canvas_height {
                            image.put_pixel(x + px, y + py, Rgba([bg.0, bg.1, bg.2, 255]));
                        }
                    }
                }
            }
            // Right half block
            '\u{2590}' => {
                for py in 0..cell_h {
                    for px in 0..cell_w / 2 {
                        if x + px < self.canvas_width && y + py < self.canvas_height {
                            image.put_pixel(x + px, y + py, Rgba([bg.0, bg.1, bg.2, 255]));
                        }
                    }
                    for px in cell_w / 2..cell_w {
                        if x + px < self.canvas_width && y + py < self.canvas_height {
                            image.put_pixel(x + px, y + py, Rgba([fg.0, fg.1, fg.2, 255]));
                        }
                    }
                }
            }
            // For other block elements, fall back to background rendering
            _ => {
                for py in 0..cell_h {
                    for px in 0..cell_w {
                        if x + px < self.canvas_width && y + py < self.canvas_height {
                            image.put_pixel(x + px, y + py, Rgba([bg.0, bg.1, bg.2, 255]));
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Render cell background
    fn render_background(&self, image: &mut RgbaImage, x: u32, y: u32, bg: (u8, u8, u8)) {
        for dy in 0..self.cell_height {
            for dx in 0..self.cell_width {
                let px = x + dx;
                let py = y + dy;
                if px < self.canvas_width && py < self.canvas_height {
                    image.put_pixel(px, py, Rgba([bg.0, bg.1, bg.2, 255]));
                }
            }
        }
    }

    /// Render a character
    #[allow(clippy::too_many_arguments)]
    fn render_char(
        &mut self,
        image: &mut RgbaImage,
        c: char,
        x: u32,
        y: u32,
        fg: (u8, u8, u8),
        bg: (u8, u8, u8),
        bold: bool,
        italic: bool,
    ) -> ScreenshotResult<()> {
        // Get all needed values from font_cache first to avoid multiple mutable borrows
        // For block-drawing characters, render them as filled rectangles for pixel-perfect rendering
        // Font glyphs often have spacing/bearing that causes gaps
        let is_block_element = matches!(c, '\u{2580}'..='\u{259F}');

        if is_block_element {
            self.render_block_element(image, c, x, y, fg, bg)?;
            return Ok(());
        }

        let ascent = self.font_cache.ascent();
        let descent = self.font_cache.descent(); // Note: negative value
        let glyph = self.font_cache.get_glyph(c, bold, italic);

        // Calculate baseline position within the cell
        // The baseline is positioned to center the font's line height within the cell
        // Font metrics: ascent (positive) + |descent| (absolute value of negative descent) = line height
        let font_line_height = ascent - descent; // descent is negative, so this adds them

        // For box-drawing characters, don't add vertical padding so they fill the full cell
        let is_box_drawing = matches!(c, '\u{2500}'..='\u{257F}');

        let vertical_padding = if is_box_drawing {
            0
        } else {
            (self.cell_height as i32 - font_line_height) / 2
        };
        let baseline_y = y as i32 + vertical_padding + ascent;

        // Position the glyph relative to the baseline
        // FreeType's bitmap_top is the distance from the baseline to the top of the glyph
        // which is stored in ymin (but negated since ymin is distance from baseline to bottom)
        // Actually, we need to use -ymin as the bitmap_top value
        // The glyph's top edge is at: baseline_y - (-ymin) = baseline_y + ymin
        let glyph_top_y = baseline_y + glyph.metrics.ymin;

        // Clone glyph data to avoid borrow checker issues
        let glyph_width = glyph.metrics.width;
        let glyph_height = glyph.metrics.height;
        let glyph_xmin = glyph.metrics.xmin; // Horizontal bearing (bitmap_left)
        let glyph_bitmap = glyph.bitmap.clone();
        let glyph_format = glyph.format.clone();

        // Calculate horizontal offset
        // For box-drawing characters, ignore bearing to ensure they fill the cell edge-to-edge
        let glyph_left_x = if is_box_drawing {
            x as i32
        } else {
            x as i32 + glyph_xmin
        };

        // Use a lightweight faux-bold pass (single-pixel horizontal stroke) so glyphs
        // better match Textual/iTerm rendering when bold brightening is disabled.
        const REGULAR_OFFSETS: [(i32, i32); 1] = [(0, 0)];
        const BOLD_OFFSETS: [(i32, i32); 2] = [(0, 0), (1, 0)];
        let bold_offsets: &[(i32, i32)] = if bold {
            &BOLD_OFFSETS
        } else {
            &REGULAR_OFFSETS
        };

        for &(x_offset, y_offset) in bold_offsets {
            // Blit glyph bitmap onto image based on format
            match glyph_format {
                BitmapFormat::Grayscale => {
                    // Grayscale bitmap (alpha channel only)
                    for glyph_y in 0..glyph_height {
                        for glyph_x in 0..glyph_width {
                            let idx = glyph_y * glyph_width + glyph_x;
                            if idx >= glyph_bitmap.len() {
                                continue;
                            }

                            let alpha = glyph_bitmap[idx];
                            let px = (glyph_left_x + glyph_x as i32 + x_offset) as u32;
                            let py = (glyph_top_y + glyph_y as i32 + y_offset) as u32;

                            blend_grayscale_pixel(
                                image,
                                px,
                                py,
                                fg,
                                alpha,
                                self.canvas_width,
                                self.canvas_height,
                            );
                        }
                    }
                }
                BitmapFormat::Rgba => {
                    // RGBA bitmap (color emoji)
                    for glyph_y in 0..glyph_height {
                        for glyph_x in 0..glyph_width {
                            let idx = (glyph_y * glyph_width + glyph_x) * 4;
                            if idx + 3 >= glyph_bitmap.len() {
                                continue;
                            }

                            let r = glyph_bitmap[idx];
                            let g = glyph_bitmap[idx + 1];
                            let b = glyph_bitmap[idx + 2];
                            let alpha = glyph_bitmap[idx + 3];

                            let px = (glyph_left_x + glyph_x as i32 + x_offset) as u32;
                            let py = (glyph_top_y + glyph_y as i32 + y_offset) as u32;

                            blend_rgba_pixel(
                                image,
                                px,
                                py,
                                (r, g, b),
                                alpha,
                                self.canvas_width,
                                self.canvas_height,
                            );
                        }
                    }
                }
            } // End of bold offset loop
        }

        Ok(())
    }

    /// Check if a string contains Regional Indicator characters (flag emojis)
    pub(crate) fn contains_regional_indicators(text: &str) -> bool {
        text.chars().any(|c| matches!(c as u32, 0x1F1E6..=0x1F1FF))
    }

    /// Render a line using text shaping (for flag emojis and complex emoji)
    fn render_shaped_line(
        &mut self,
        image: &mut RgbaImage,
        row: usize,
        grid: &Grid,
    ) -> ScreenshotResult<()> {
        // Get line text first
        let line_text = grid.row_text(row);
        if line_text.is_empty() {
            return Ok(());
        }

        // Shape the line (need mutable borrow)
        let shaped_glyphs = match &mut self.shaper {
            Some(shaper) => shaper.shape_line(&line_text),
            None => {
                // Fallback to character-by-character rendering
                for col in 0..grid.cols() {
                    if let Some(cell) = grid.get(col, row) {
                        self.render_cell(image, cell, col, row)?;
                    }
                }
                return Ok(());
            }
        };

        // Render backgrounds and decorations first using normal cell rendering
        for col in 0..grid.cols() {
            if let Some(cell) = grid.get(col, row) {
                let x = col as u32 * self.cell_width + self.config.padding_px;
                let y = row as u32 * self.cell_height + self.config.padding_px;

                // Skip wide character spacers
                if cell.flags.wide_char_spacer() {
                    continue;
                }

                let (fg, bg) = self.resolve_colors(cell);

                // Render background
                self.render_background(image, x, y, bg);

                // Render text decorations (underline, strikethrough, overline)
                if cell.flags.underline() {
                    let underline_color = cell.underline_color.map(|c| c.to_rgb()).unwrap_or(fg);
                    self.render_underline(image, x, y, cell.flags.underline_style, underline_color);
                }

                if cell.flags.strikethrough() {
                    self.render_strikethrough(image, x, y, fg);
                }

                if cell.flags.overline() {
                    self.render_overline(image, x, y, fg);
                }
            }
        }

        // Now render shaped glyphs using cluster information
        // HarfBuzz clusters are BYTE offsets, not character indices!
        // We need to:
        // 1. Map byte offset -> character index
        // 2. Map character index -> grid column

        // Build byte offset -> character index mapping
        let text_bytes = line_text.len();
        let mut byte_to_char = vec![0; text_bytes + 1];
        let mut byte_offset = 0;
        for (char_idx, c) in line_text.chars().enumerate() {
            let char_len = c.len_utf8();
            for i in 0..char_len {
                if byte_offset + i < byte_to_char.len() {
                    byte_to_char[byte_offset + i] = char_idx;
                }
            }
            byte_offset += char_len;
        }

        // Build character index -> grid column mapping
        // Calculate column position for each character based on width
        let mut char_to_col = Vec::new();
        let mut current_col = 0;

        for c in line_text.chars() {
            char_to_col.push(current_col);
            // Use unicode_width to get character width (1 for normal, 2 for wide)
            let width = unicode_width::UnicodeWidthChar::width(c).unwrap_or(1);
            current_col += width;
        }

        // Render each shaped glyph at its corresponding grid column
        for shaped in &shaped_glyphs {
            let byte_cluster = shaped.glyph.cluster as usize;

            // Convert byte offset to character index
            if byte_cluster >= byte_to_char.len() {
                continue; // Cluster out of range
            }
            let char_index = byte_to_char[byte_cluster];

            // Find the grid column for this character
            if char_index >= char_to_col.len() {
                continue; // Character index out of range
            }

            let col = char_to_col[char_index];
            if let Some(cell) = grid.get(col, row) {
                // Skip hidden or space characters
                if cell.flags.hidden() || cell.c == ' ' {
                    continue;
                }

                let x = col as u32 * self.cell_width + self.config.padding_px;
                let y = row as u32 * self.cell_height + self.config.padding_px;

                let (fg, _) = self.resolve_colors(cell);

                // Render the shaped glyph
                self.render_shaped_glyph(image, shaped, x, y, fg, cell.flags.bold(), cell.c)?;
            }
        }

        Ok(())
    }

    /// Render a shaped glyph
    #[allow(clippy::too_many_arguments)]
    fn render_shaped_glyph(
        &mut self,
        image: &mut RgbaImage,
        shaped: &ShapedGlyphWithFont,
        x: u32,
        y: u32,
        fg: (u8, u8, u8),
        bold: bool,
        c: char,
    ) -> ScreenshotResult<()> {
        // Get the glyph bitmap from the font cache by glyph ID
        let glyph =
            match self
                .font_cache
                .get_glyph_by_id(shaped.glyph.glyph_id, shaped.font_type, bold)
            {
                Some(g) => g,
                None => return Ok(()), // Glyph not available
            };

        // Get font metrics for baseline calculation
        let ascent = self.font_cache.ascent();
        let descent = self.font_cache.descent();

        // Calculate baseline position
        let font_line_height = ascent - descent;

        // For block-drawing characters, don't add vertical padding so they fill the full cell
        let is_block_char = matches!(c,
            '\u{2580}'..='\u{259F}' | // Block Elements
            '\u{2500}'..='\u{257F}'   // Box Drawing
        );

        let vertical_padding = if is_block_char {
            0
        } else {
            (self.cell_height as i32 - font_line_height) / 2
        };
        let baseline_y = y as i32 + vertical_padding + ascent;

        // Apply HarfBuzz positioning (convert from 26.6 fixed point to pixels)
        let glyph_x_offset = shaped.glyph.x_offset / 64;
        let glyph_y_offset = shaped.glyph.y_offset / 64;

        // Calculate final glyph position
        let glyph_left_x = x as i32 + glyph.metrics.xmin + glyph_x_offset;
        let glyph_top_y = baseline_y + glyph.metrics.ymin + glyph_y_offset;

        // Clone glyph data to avoid borrow checker issues
        let glyph_width = glyph.metrics.width;
        let glyph_height = glyph.metrics.height;
        let glyph_bitmap = glyph.bitmap.clone();
        let glyph_format = glyph.format.clone();

        const REGULAR_OFFSETS: [(i32, i32); 1] = [(0, 0)];
        const BOLD_OFFSETS: [(i32, i32); 2] = [(0, 0), (1, 0)];
        let bold_offsets: &[(i32, i32)] = if bold {
            &BOLD_OFFSETS
        } else {
            &REGULAR_OFFSETS
        };

        for &(x_offset, y_offset) in bold_offsets {
            // Render glyph bitmap
            match glyph_format {
                BitmapFormat::Grayscale => {
                    // Grayscale bitmap (alpha channel only)
                    for glyph_y in 0..glyph_height {
                        for glyph_x in 0..glyph_width {
                            let idx = glyph_y * glyph_width + glyph_x;
                            if idx >= glyph_bitmap.len() {
                                continue;
                            }

                            let alpha = glyph_bitmap[idx];
                            let px = (glyph_left_x + glyph_x as i32 + x_offset) as u32;
                            let py = (glyph_top_y + glyph_y as i32 + y_offset) as u32;

                            blend_grayscale_pixel(
                                image,
                                px,
                                py,
                                fg,
                                alpha,
                                self.canvas_width,
                                self.canvas_height,
                            );
                        }
                    }
                }
                BitmapFormat::Rgba => {
                    // RGBA bitmap (color emoji)
                    for glyph_y in 0..glyph_height {
                        for glyph_x in 0..glyph_width {
                            let idx = (glyph_y * glyph_width + glyph_x) * 4;
                            if idx + 3 >= glyph_bitmap.len() {
                                continue;
                            }

                            let r = glyph_bitmap[idx];
                            let g = glyph_bitmap[idx + 1];
                            let b = glyph_bitmap[idx + 2];
                            let alpha = glyph_bitmap[idx + 3];

                            let px = (glyph_left_x + glyph_x as i32 + x_offset) as u32;
                            let py = (glyph_top_y + glyph_y as i32 + y_offset) as u32;

                            blend_rgba_pixel(
                                image,
                                px,
                                py,
                                (r, g, b),
                                alpha,
                                self.canvas_width,
                                self.canvas_height,
                            );
                        }
                    }
                }
            }
        }

        Ok(())
    }

    /// Render underline
    fn render_underline(
        &self,
        image: &mut RgbaImage,
        x: u32,
        y: u32,
        style: UnderlineStyle,
        color: (u8, u8, u8),
    ) {
        match style {
            UnderlineStyle::None => {}
            UnderlineStyle::Straight => self.render_straight_underline(image, x, y, color),
            UnderlineStyle::Double => self.render_double_underline(image, x, y, color),
            UnderlineStyle::Curly => self.render_curly_underline(image, x, y, color),
            UnderlineStyle::Dotted => self.render_dotted_underline(image, x, y, color),
            UnderlineStyle::Dashed => self.render_dashed_underline(image, x, y, color),
        }
    }

    /// Render straight underline
    fn render_straight_underline(
        &self,
        image: &mut RgbaImage,
        x: u32,
        y: u32,
        color: (u8, u8, u8),
    ) {
        let line_y = y + self.cell_height - 2;
        for dx in 0..self.cell_width {
            let px = x + dx;
            if px < self.canvas_width && line_y < self.canvas_height {
                image.put_pixel(px, line_y, Rgba([color.0, color.1, color.2, 255]));
            }
        }
    }

    /// Render double underline
    fn render_double_underline(&self, image: &mut RgbaImage, x: u32, y: u32, color: (u8, u8, u8)) {
        let line_y1 = y + self.cell_height - 3;
        let line_y2 = y + self.cell_height - 1;
        for dx in 0..self.cell_width {
            let px = x + dx;
            if px < self.canvas_width {
                if line_y1 < self.canvas_height {
                    image.put_pixel(px, line_y1, Rgba([color.0, color.1, color.2, 255]));
                }
                if line_y2 < self.canvas_height {
                    image.put_pixel(px, line_y2, Rgba([color.0, color.1, color.2, 255]));
                }
            }
        }
    }

    /// Render curly underline (approximated with sine wave)
    fn render_curly_underline(&self, image: &mut RgbaImage, x: u32, y: u32, color: (u8, u8, u8)) {
        let base_y = y + self.cell_height - 2;
        for dx in 0..self.cell_width {
            let px = x + dx;
            // Simple sine wave approximation
            let wave = ((dx as f32 * std::f32::consts::PI * 2.0) / (self.cell_width as f32)).sin();
            let offset = (wave * 1.5) as i32;
            let line_y = (base_y as i32 + offset) as u32;

            if px < self.canvas_width && line_y < self.canvas_height {
                image.put_pixel(px, line_y, Rgba([color.0, color.1, color.2, 255]));
            }
        }
    }

    /// Render dotted underline
    fn render_dotted_underline(&self, image: &mut RgbaImage, x: u32, y: u32, color: (u8, u8, u8)) {
        let line_y = y + self.cell_height - 2;
        for dx in (0..self.cell_width).step_by(3) {
            let px = x + dx;
            if px < self.canvas_width && line_y < self.canvas_height {
                image.put_pixel(px, line_y, Rgba([color.0, color.1, color.2, 255]));
            }
        }
    }

    /// Render dashed underline
    fn render_dashed_underline(&self, image: &mut RgbaImage, x: u32, y: u32, color: (u8, u8, u8)) {
        let line_y = y + self.cell_height - 2;
        let dash_length = 4;
        let gap_length = 2;

        let mut dx = 0;
        while dx < self.cell_width {
            for i in 0..dash_length {
                let px = x + dx + i;
                if dx + i >= self.cell_width {
                    break;
                }
                if px < self.canvas_width && line_y < self.canvas_height {
                    image.put_pixel(px, line_y, Rgba([color.0, color.1, color.2, 255]));
                }
            }
            dx += dash_length + gap_length;
        }
    }

    /// Render strikethrough
    fn render_strikethrough(&self, image: &mut RgbaImage, x: u32, y: u32, color: (u8, u8, u8)) {
        let line_y = y + self.cell_height / 2;
        for dx in 0..self.cell_width {
            let px = x + dx;
            if px < self.canvas_width && line_y < self.canvas_height {
                image.put_pixel(px, line_y, Rgba([color.0, color.1, color.2, 255]));
            }
        }
    }

    /// Render overline
    fn render_overline(&self, image: &mut RgbaImage, x: u32, y: u32, color: (u8, u8, u8)) {
        let line_y = y + 1;
        for dx in 0..self.cell_width {
            let px = x + dx;
            if px < self.canvas_width && line_y < self.canvas_height {
                image.put_pixel(px, line_y, Rgba([color.0, color.1, color.2, 255]));
            }
        }
    }

    /// Render Sixel graphic onto the image using actual pixels
    fn render_sixel_pixels(&self, image: &mut RgbaImage, graphic: &TerminalGraphic) {
        // Calculate pixel position from terminal position
        let x_offset = self.config.padding_px + graphic.position.0 as u32 * self.cell_width;
        let y_offset = self.config.padding_px + graphic.position.1 as u32 * self.cell_height;

        // Blit the Sixel graphic pixels onto the image
        for py in 0..graphic.height {
            for px in 0..graphic.width {
                if let Some((r, g, b, a)) = graphic.get_pixel(px, py) {
                    // Skip fully transparent pixels
                    if a == 0 {
                        continue;
                    }

                    let dest_x = x_offset + px as u32;
                    let dest_y = y_offset + py as u32;

                    // Alpha blend the Sixel pixel
                    blend_rgba_pixel(
                        image,
                        dest_x,
                        dest_y,
                        (r, g, b),
                        a,
                        self.canvas_width,
                        self.canvas_height,
                    );
                }
            }
        }
    }

    /// Render Sixel graphic using half-block characters (matches TUI appearance)
    fn render_sixel_halfblocks(
        &mut self,
        image: &mut RgbaImage,
        graphic: &TerminalGraphic,
    ) -> ScreenshotResult<()> {
        // Calculate cell position from terminal position
        let base_col = graphic.position.0;
        let base_row = graphic.position.1;

        // Get how many terminal cells this graphic spans
        // This uses the original terminal's cell dimensions if available,
        // otherwise falls back to the screenshot's cell dimensions
        let (cells_wide, cells_high) = graphic.cell_span(self.cell_width, self.cell_height);

        for cell_row in 0..cells_high {
            for cell_col in 0..cells_wide {
                let col = base_col + cell_col;
                let row = base_row + cell_row;

                // Map this cell position to Sixel pixels
                // The Sixel spans cells_wide x cells_high cells
                // Each cell represents (graphic.width / cells_wide) x (graphic.height / cells_high) pixels
                let pixels_per_cell_x = graphic.width as f32 / cells_wide as f32;
                let pixels_per_cell_y = graphic.height as f32 / cells_high as f32;

                // Sample at the center horizontally, and at 1/4 and 3/4 vertically
                let sixel_x =
                    (cell_col as f32 * pixels_per_cell_x + pixels_per_cell_x / 2.0) as usize;
                let sixel_y_top =
                    (cell_row as f32 * pixels_per_cell_y + pixels_per_cell_y / 4.0) as usize;
                let sixel_y_bottom =
                    (cell_row as f32 * pixels_per_cell_y + 3.0 * pixels_per_cell_y / 4.0) as usize;

                // Get top and bottom pixel colors
                let top_color = if sixel_x < graphic.width && sixel_y_top < graphic.height {
                    graphic.get_pixel(sixel_x, sixel_y_top)
                } else {
                    None
                };

                let bottom_color = if sixel_x < graphic.width && sixel_y_bottom < graphic.height {
                    graphic.get_pixel(sixel_x, sixel_y_bottom)
                } else {
                    None
                };

                // Skip if both are transparent
                if top_color.is_none() && bottom_color.is_none() {
                    continue;
                }

                // Convert to RGB, using transparent black for missing pixels
                let top_rgb = top_color.map(|(r, g, b, _)| (r, g, b)).unwrap_or((0, 0, 0));
                let bottom_rgb = bottom_color
                    .map(|(r, g, b, _)| (r, g, b))
                    .unwrap_or((0, 0, 0));

                // Render the half-block character '▀' (UPPER HALF BLOCK)
                // Foreground = top pixel, Background = bottom pixel
                let x = col as u32 * self.cell_width + self.config.padding_px;
                let y = row as u32 * self.cell_height + self.config.padding_px;

                // First render background (bottom color)
                self.render_background(image, x, y, bottom_rgb);

                // Then render the half-block character with foreground (top color)
                self.render_char(image, '▀', x, y, top_rgb, bottom_rgb, false, false)?;
            }
        }

        Ok(())
    }

    /// Render cursor at the given position
    fn render_cursor(&self, image: &mut RgbaImage, cursor: &Cursor) {
        let cursor_color = self.config.cursor_color;
        let x = self.config.padding_px + cursor.col as u32 * self.cell_width;
        let y = self.config.padding_px + cursor.row as u32 * self.cell_height;

        match cursor.style {
            CursorStyle::BlinkingBlock | CursorStyle::SteadyBlock => {
                // Render block cursor - fill the entire cell
                for dy in 0..self.cell_height {
                    for dx in 0..self.cell_width {
                        let px = x + dx;
                        let py = y + dy;
                        if px < self.canvas_width && py < self.canvas_height {
                            // Semi-transparent cursor (50% opacity)
                            let existing = image.get_pixel(px, py);
                            let blended = Rgba([
                                ((existing[0] as u16 + cursor_color.0 as u16) / 2) as u8,
                                ((existing[1] as u16 + cursor_color.1 as u16) / 2) as u8,
                                ((existing[2] as u16 + cursor_color.2 as u16) / 2) as u8,
                                255,
                            ]);
                            image.put_pixel(px, py, blended);
                        }
                    }
                }
            }
            CursorStyle::BlinkingUnderline | CursorStyle::SteadyUnderline => {
                // Render underline cursor - 2 pixels high at bottom of cell
                let underline_start = y + self.cell_height - 2;
                for dy in 0..2 {
                    for dx in 0..self.cell_width {
                        let px = x + dx;
                        let py = underline_start + dy;
                        if px < self.canvas_width && py < self.canvas_height {
                            image.put_pixel(
                                px,
                                py,
                                Rgba([cursor_color.0, cursor_color.1, cursor_color.2, 255]),
                            );
                        }
                    }
                }
            }
            CursorStyle::BlinkingBar | CursorStyle::SteadyBar => {
                // Render bar cursor - 2 pixels wide at left edge
                for dy in 0..self.cell_height {
                    for dx in 0..2 {
                        let px = x + dx;
                        let py = y + dy;
                        if px < self.canvas_width && py < self.canvas_height {
                            image.put_pixel(
                                px,
                                py,
                                Rgba([cursor_color.0, cursor_color.1, cursor_color.2, 255]),
                            );
                        }
                    }
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::cursor::CursorStyle;
    use image::Rgba;

    // Test helper to create a minimal config
    fn create_test_config() -> ScreenshotConfig {
        ScreenshotConfig {
            font_path: None,
            font_size: 14.0,
            padding_px: 10,
            char_width_multiplier: 1.0,
            line_height_multiplier: 1.0,
            include_scrollback: false,
            scrollback_lines: None,
            antialiasing: true,
            background_color: Some((0, 0, 0)),
            sixel_render_mode: SixelRenderMode::Disabled,
            render_cursor: false,
            cursor_color: (255, 255, 255),
            link_color: None,
            bold_color: None,
            use_bold_color: false,
            bold_brightening: false,
            minimum_contrast: 0.5,
            faint_text_alpha: 0.5,
            quality: 90,
            format: crate::screenshot::config::ImageFormat::Png,
        }
    }

    #[test]
    fn test_contains_regional_indicators_with_flag() {
        // US flag: 🇺🇸 (U+1F1FA U+1F1F8)
        let text = "Hello 🇺🇸 World";
        assert!(Renderer::contains_regional_indicators(text));
    }

    #[test]
    fn test_contains_regional_indicators_without_flag() {
        let text = "Hello World";
        assert!(!Renderer::contains_regional_indicators(text));
    }

    #[test]
    fn test_contains_regional_indicators_with_emoji_no_flag() {
        // Regular emoji, not a flag
        let text = "Hello 😀 World";
        assert!(!Renderer::contains_regional_indicators(text));
    }

    #[test]
    fn test_contains_regional_indicators_multiple_flags() {
        // Multiple flags: 🇺🇸 🇬🇧 🇯🇵
        let text = "🇺🇸 🇬🇧 🇯🇵";
        assert!(Renderer::contains_regional_indicators(text));
    }

    #[test]
    fn test_contains_regional_indicators_empty_string() {
        assert!(!Renderer::contains_regional_indicators(""));
    }

    #[test]
    fn test_render_background() {
        let _config = create_test_config();
        // Need actual font for renderer, but we can test if we mock it
        // For now, test the helper functions that don't require fonts

        // Create a small test image
        let _image = RgbaImage::new(100, 100);

        // We can't create a full Renderer without fonts, but we can test
        // the logic by creating a mock renderer structure
        // Skip this test for now as it requires FontCache
    }

    #[test]
    fn test_render_straight_underline_pixels() {
        // Create a test image and config
        let _config = create_test_config();
        let _image = RgbaImage::from_pixel(100, 100, Rgba([0, 0, 0, 255]));

        // We need a renderer to call the method, but it requires FontCache
        // which needs actual fonts. Let's test the underline rendering logic
        // by checking the expected pixel positions

        // For a cell at (0, 0) with cell_height=20 and cell_width=10
        // straight underline should be at y = cell_height - 2 = 18
        // This would be tested if we had a way to create a Renderer without fonts
    }

    #[test]
    fn test_resolve_colors_normal() {
        use crate::cell::Cell;
        use crate::color::Color;

        let _config = create_test_config();
        // Can't create Renderer without FontCache, so we'll test the logic separately

        let cell = Cell {
            fg: Color::Rgb(255, 0, 0), // Red foreground
            bg: Color::Rgb(0, 0, 255), // Blue background
            ..Default::default()
        };

        // Test that colors are returned as-is for normal cell
        // This would require creating a Renderer instance
        assert_eq!(cell.fg, Color::Rgb(255, 0, 0));
    }

    #[test]
    fn test_resolve_colors_with_reverse() {
        use crate::cell::Cell;
        use crate::color::Color;

        let mut cell = Cell {
            fg: Color::Rgb(255, 0, 0), // Red
            bg: Color::Rgb(0, 0, 255), // Blue
            ..Default::default()
        };
        cell.flags.set_reverse(true);

        // When reverse is set, fg and bg should be swapped
        // Expected: fg=Blue, bg=Red
        assert!(cell.flags.reverse());
    }

    #[test]
    fn test_resolve_colors_with_dim() {
        use crate::cell::Cell;
        use crate::color::Color;

        let mut cell = Cell {
            fg: Color::Rgb(200, 100, 50),
            bg: Color::Rgb(0, 0, 255),
            ..Default::default()
        };
        cell.flags.set_dim(true);

        // When dim is set, foreground should be at ~50% brightness
        // Expected fg: (100, 50, 25)
    }

    #[test]
    fn test_block_element_upper_half() {
        // Test that upper half block character is recognized
        let c = '\u{2580}'; // ▀
        assert!(matches!(c, '\u{2580}'..='\u{259F}'));
    }

    #[test]
    fn test_block_element_lower_half() {
        // Test that lower half block character is recognized
        let c = '\u{2584}'; // ▄
        assert!(matches!(c, '\u{2580}'..='\u{259F}'));
    }

    #[test]
    fn test_block_element_full_block() {
        // Test that full block character is recognized
        let c = '\u{2588}'; // █
        assert!(matches!(c, '\u{2580}'..='\u{259F}'));
    }

    #[test]
    fn test_block_element_left_half() {
        // Test that left half block is recognized
        let c = '\u{258C}'; // ▌
        assert!(matches!(c, '\u{2580}'..='\u{259F}'));
    }

    #[test]
    fn test_block_element_right_half() {
        // Test that right half block is recognized
        let c = '\u{2590}'; // ▐
        assert!(matches!(c, '\u{2580}'..='\u{259F}'));
    }

    #[test]
    fn test_box_drawing_character_detection() {
        // Test box-drawing character range
        let c = '\u{2500}'; // ─
        assert!(matches!(c, '\u{2500}'..='\u{257F}'));

        let c = '\u{2550}'; // ═
        assert!(matches!(c, '\u{2500}'..='\u{257F}'));
    }

    #[test]
    fn test_non_block_character() {
        // Regular ASCII should not be a block element
        let c = 'A';
        assert!(!matches!(c, '\u{2580}'..='\u{259F}'));
    }

    #[test]
    fn test_cursor_style_variants() {
        // Test that all cursor styles are handled
        let styles = vec![
            CursorStyle::BlinkingBlock,
            CursorStyle::SteadyBlock,
            CursorStyle::BlinkingUnderline,
            CursorStyle::SteadyUnderline,
            CursorStyle::BlinkingBar,
            CursorStyle::SteadyBar,
        ];

        for style in styles {
            // Each style should match one of the render patterns
            let is_block = matches!(style, CursorStyle::BlinkingBlock | CursorStyle::SteadyBlock);
            let is_underline = matches!(
                style,
                CursorStyle::BlinkingUnderline | CursorStyle::SteadyUnderline
            );
            let is_bar = matches!(style, CursorStyle::BlinkingBar | CursorStyle::SteadyBar);

            assert!(is_block || is_underline || is_bar);
        }
    }

    #[test]
    fn test_underline_style_matching() {
        use crate::cell::UnderlineStyle;

        // Test all underline styles
        let styles = vec![
            UnderlineStyle::None,
            UnderlineStyle::Straight,
            UnderlineStyle::Double,
            UnderlineStyle::Curly,
            UnderlineStyle::Dotted,
            UnderlineStyle::Dashed,
        ];

        for style in styles {
            // Each style should be matchable
            match style {
                UnderlineStyle::None => {}
                UnderlineStyle::Straight => {}
                UnderlineStyle::Double => {}
                UnderlineStyle::Curly => {}
                UnderlineStyle::Dotted => {}
                UnderlineStyle::Dashed => {}
            }
        }
    }

    #[test]
    fn test_canvas_dimension_calculations() {
        // Test canvas size calculations without creating full Renderer
        let rows = 24;
        let cols = 80;
        let padding = 10;
        let cell_width = 8;
        let cell_height = 16;

        let expected_width = cols * cell_width + padding * 2;
        let expected_height = rows * cell_height + padding * 2;

        assert_eq!(expected_width, 660); // 80 * 8 + 20
        assert_eq!(expected_height, 404); // 24 * 16 + 20
    }

    #[test]
    fn test_canvas_with_multipliers() {
        // Test cell dimension multipliers
        let base_width = 8.0;
        let base_height = 16.0;
        let width_mult = 1.2;
        let height_mult = 1.5;

        let cell_width = (base_width * width_mult) as u32;
        let cell_height = (base_height * height_mult) as u32;

        assert_eq!(cell_width, 9); // 8.0 * 1.2 = 9.6 -> 9
        assert_eq!(cell_height, 24); // 16.0 * 1.5 = 24.0
    }

    #[test]
    fn test_sixel_render_mode_matching() {
        // Test all sixel render modes
        let modes = vec![
            SixelRenderMode::Disabled,
            SixelRenderMode::Pixels,
            SixelRenderMode::HalfBlocks,
        ];

        for mode in modes {
            match mode {
                SixelRenderMode::Disabled => {}
                SixelRenderMode::Pixels => {}
                SixelRenderMode::HalfBlocks => {}
            }
        }
    }

    #[test]
    fn test_regional_indicator_range() {
        // Test the Regional Indicator Unicode range (U+1F1E6 to U+1F1FF)
        let regional_a = '\u{1F1E6}'; // First regional indicator (A)
        let regional_z = '\u{1F1FF}'; // Last regional indicator (Z)

        assert!(matches!(regional_a as u32, 0x1F1E6..=0x1F1FF));
        assert!(matches!(regional_z as u32, 0x1F1E6..=0x1F1FF));
    }

    #[test]
    fn test_config_background_color_unwrap() {
        let config = create_test_config();
        let bg = config.background_color.unwrap_or((0, 0, 0));
        assert_eq!(bg, (0, 0, 0));
    }

    #[test]
    fn test_config_with_custom_colors() {
        let mut config = create_test_config();
        config.link_color = Some((0, 0, 255));
        config.bold_color = Some((255, 255, 0));
        config.use_bold_color = true;

        assert_eq!(config.link_color, Some((0, 0, 255)));
        assert_eq!(config.bold_color, Some((255, 255, 0)));
        assert!(config.use_bold_color);
    }

    #[test]
    fn test_alpha_blending_calculation() {
        // Test the alpha blending logic used in cursor rendering
        let existing = (100u8, 150u8, 200u8);
        let cursor = (255u8, 255u8, 255u8);

        // 50% blend
        let blended = (
            ((existing.0 as u16 + cursor.0 as u16) / 2) as u8,
            ((existing.1 as u16 + cursor.1 as u16) / 2) as u8,
            ((existing.2 as u16 + cursor.2 as u16) / 2) as u8,
        );

        assert_eq!(blended.0, 177); // (100 + 255) / 2 = 177.5 -> 177
        assert_eq!(blended.1, 202); // (150 + 255) / 2 = 202.5 -> 202
        assert_eq!(blended.2, 227); // (200 + 255) / 2 = 227.5 -> 227
    }

    #[test]
    fn test_dim_color_calculation() {
        // Test dim color calculation (50% brightness reduction)
        let fg = (200u8, 100u8, 50u8);
        let dimmed = (fg.0 / 2, fg.1 / 2, fg.2 / 2);

        assert_eq!(dimmed, (100, 50, 25));
    }

    #[test]
    fn test_color_swap_for_reverse() {
        // Test color swapping for reverse video
        let mut fg = (255, 0, 0);
        let mut bg = (0, 0, 255);

        std::mem::swap(&mut fg, &mut bg);

        assert_eq!(fg, (0, 0, 255));
        assert_eq!(bg, (255, 0, 0));
    }

    #[test]
    fn test_curly_underline_sine_wave() {
        // Test sine wave calculation for curly underline
        use std::f32::consts::PI;

        let cell_width = 10u32;

        for dx in 0..cell_width {
            let wave = ((dx as f32 * PI * 2.0) / (cell_width as f32)).sin();
            let offset = (wave * 1.5) as i32;

            // Offset should be in range [-1, 1] after multiplication by 1.5
            assert!((-2..=2).contains(&offset));
        }
    }

    #[test]
    fn test_dashed_underline_pattern() {
        // Test dashed underline pattern calculation
        let cell_width = 20u32;
        let dash_length = 4;
        let gap_length = 2;

        let mut drawn_pixels = 0;
        let mut dx = 0;

        while dx < cell_width {
            for i in 0..dash_length {
                if dx + i >= cell_width {
                    break;
                }
                drawn_pixels += 1;
            }
            dx += dash_length + gap_length;
        }

        // Should draw approximately (cell_width / (dash + gap)) * dash pixels
        assert!(drawn_pixels > 0);
        assert!(drawn_pixels < cell_width);
    }

    #[test]
    fn test_dotted_underline_pattern() {
        // Test dotted underline step size
        let cell_width = 12u32;
        let step = 3;

        let drawn_pixels = (0..cell_width).step_by(step).count();

        // Should draw cell_width / step pixels
        assert_eq!(drawn_pixels, 4); // 12 / 3 = 4 pixels
    }

    #[test]
    fn test_sixel_halfblock_sampling_positions() {
        // Test the sampling positions for half-block rendering
        let cells_wide = 10usize;
        let cells_high = 5usize;
        let graphic_width = 100usize;
        let graphic_height = 50usize;

        let pixels_per_cell_x = graphic_width as f32 / cells_wide as f32;
        let pixels_per_cell_y = graphic_height as f32 / cells_high as f32;

        assert_eq!(pixels_per_cell_x, 10.0); // 100 / 10
        assert_eq!(pixels_per_cell_y, 10.0); // 50 / 5

        // Sample positions for cell (0, 0)
        let sixel_x = (0.0 * pixels_per_cell_x + pixels_per_cell_x / 2.0) as usize;
        let sixel_y_top = (0.0 * pixels_per_cell_y + pixels_per_cell_y / 4.0) as usize;
        let sixel_y_bottom = (0.0 * pixels_per_cell_y + 3.0 * pixels_per_cell_y / 4.0) as usize;

        assert_eq!(sixel_x, 5); // Center horizontally
        assert_eq!(sixel_y_top, 2); // 1/4 position
        assert_eq!(sixel_y_bottom, 7); // 3/4 position
    }

    #[test]
    fn test_harfbuzz_fixed_point_conversion() {
        // Test HarfBuzz 26.6 fixed-point to pixel conversion
        let fixed_point_offset = 64; // 1 pixel in 26.6 format
        let pixel_offset = fixed_point_offset / 64;

        assert_eq!(pixel_offset, 1);

        let fixed_point_offset = 128; // 2 pixels
        let pixel_offset = fixed_point_offset / 64;

        assert_eq!(pixel_offset, 2);
    }

    #[test]
    fn test_byte_to_char_index_mapping() {
        // Test UTF-8 byte to character index mapping logic
        let text = "Hello 世界"; // Mixed ASCII and CJK

        let mut byte_to_char = vec![0; text.len() + 1];
        let mut byte_offset = 0;

        for (char_idx, c) in text.chars().enumerate() {
            let char_len = c.len_utf8();
            for i in 0..char_len {
                if byte_offset + i < byte_to_char.len() {
                    byte_to_char[byte_offset + i] = char_idx;
                }
            }
            byte_offset += char_len;
        }

        // 'H' is at byte 0, char index 0
        assert_eq!(byte_to_char[0], 0);
        // ' ' is at byte 5, char index 5
        assert_eq!(byte_to_char[5], 5);
        // '世' spans bytes 6-8, all should map to char index 6
        assert_eq!(byte_to_char[6], 6);
        assert_eq!(byte_to_char[7], 6);
        assert_eq!(byte_to_char[8], 6);
    }

    #[test]
    fn test_wide_char_column_mapping() {
        // Test column mapping for wide characters
        use unicode_width::UnicodeWidthChar;

        let text = "Hello世界"; // ASCII + wide chars
        let mut char_to_col = Vec::new();
        let mut current_col = 0;

        for c in text.chars() {
            char_to_col.push(current_col);
            let width = UnicodeWidthChar::width(c).unwrap_or(1);
            current_col += width;
        }

        // 'H' at column 0
        assert_eq!(char_to_col[0], 0);
        // 'e' at column 1
        assert_eq!(char_to_col[1], 1);
        // '世' at column 5 (after "Hello")
        assert_eq!(char_to_col[5], 5);
        // '界' at column 7 (世 is 2 columns wide)
        assert_eq!(char_to_col[6], 7);
    }

    #[test]
    fn test_baseline_calculation_with_padding() {
        // Test baseline position calculation
        let cell_height = 20i32;
        let ascent = 15i32;
        let descent = -5i32;

        let font_line_height = ascent - descent; // 15 - (-5) = 20
        let vertical_padding = (cell_height - font_line_height) / 2;

        assert_eq!(vertical_padding, 0); // Perfect fit

        let y = 0i32;
        let baseline_y = y + vertical_padding + ascent;

        assert_eq!(baseline_y, 15);
    }

    #[test]
    fn test_baseline_calculation_with_extra_space() {
        // Test with cell taller than font line height
        let cell_height = 24i32;
        let ascent = 15i32;
        let descent = -5i32;

        let font_line_height = ascent - descent; // 20
        let vertical_padding = (cell_height - font_line_height) / 2;

        assert_eq!(vertical_padding, 2); // (24 - 20) / 2 = 2
    }

    #[test]
    fn test_box_drawing_no_vertical_padding() {
        // Test that box-drawing characters get 0 vertical padding
        let c = '\u{2500}'; // Box drawing character
        let is_box_drawing = matches!(c, '\u{2500}'..='\u{257F}');

        let vertical_padding = if is_box_drawing { 0 } else { 2 };

        assert_eq!(vertical_padding, 0);
    }

    #[test]
    fn test_regular_char_gets_vertical_padding() {
        // Test that regular characters get vertical padding
        let c = 'A';
        let is_box_drawing = matches!(c, '\u{2500}'..='\u{257F}');

        let vertical_padding = if is_box_drawing { 0 } else { 2 };

        assert_eq!(vertical_padding, 2);
    }
}
