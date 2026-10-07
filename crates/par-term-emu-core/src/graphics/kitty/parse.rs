//! Kitty APC parsing: chunk accumulation, key/value parsing, payload
//! assembly and decompression, and placement/graphic construction.

use super::*;

impl KittyParser {
    /// Create a new parser
    pub fn new() -> Self {
        Self::default()
    }

    /// Reset parser state for new transmission
    pub fn reset(&mut self) {
        let retain_temp_files = self.retain_temp_files;
        let allow_file_media = self.allow_file_media;
        *self = Self::default();
        self.retain_temp_files = retain_temp_files;
        self.allow_file_media = allow_file_media;
    }

    /// Parse a Kitty graphics payload
    ///
    /// Format: `key=value,key=value,...;base64data`
    pub fn parse_chunk(&mut self, payload: &str) -> Result<bool, GraphicsError> {
        // Split into params and data
        let (params_str, data_str) = payload.split_once(';').unwrap_or((payload, ""));

        // Parse key=value pairs
        for pair in params_str.split(',') {
            if let Some((key, value)) = pair.split_once('=') {
                self.params.insert(key.to_string(), value.to_string());

                match key {
                    "a" => {
                        if let Some(c) = value.chars().next() {
                            self.action = KittyAction::from_char(c).unwrap_or_default();
                        }
                    }
                    "f" => {
                        if let Ok(code) = value.parse::<u32>() {
                            self.format = KittyFormat::from_code(code).unwrap_or_default();
                        }
                    }
                    "t" => {
                        if let Some(c) = value.chars().next() {
                            self.medium = KittyMedium::from_char(c).unwrap_or_default();
                        }
                    }
                    "i" => {
                        self.image_id = value.parse().ok();
                    }
                    "p" => {
                        self.placement_id = value.parse().ok();
                    }
                    "s" => {
                        // Animation control state (for AnimationControl action) takes priority
                        if self.action == KittyAction::AnimationControl {
                            self.animation_control = AnimationControl::from_value(value);
                            debug_log!(
                                "KITTY",
                                "Parsed animation control: s={} -> {:?}",
                                value,
                                self.animation_control
                            );
                        } else {
                            // Otherwise it's width
                            self.width = value.parse().ok();
                        }
                    }
                    "v" => {
                        // v= is overloaded: height for images, num_plays for animation control
                        if self.action == KittyAction::AnimationControl {
                            // Number of times to play animation (v= for animation control)
                            // Per Kitty spec: v=0 ignored, v=1 infinite, v=N means play N times total
                            self.num_plays = value.parse().ok();
                        } else {
                            // Height for image transmission/display
                            self.height = value.parse().ok();
                        }
                    }
                    "c" => {
                        // Frame composition mode (for Frame action) takes priority
                        if self.action == KittyAction::Frame {
                            if let Some(first_char) = value.chars().next() {
                                self.frame_composition = CompositionMode::from_char(first_char);
                            }
                        } else {
                            // Otherwise it's columns
                            self.columns = value.parse().ok();
                        }
                    }
                    "r" => {
                        // Frame number (for Frame action) takes priority
                        if self.action == KittyAction::Frame {
                            self.frame_number = value.parse().ok();
                        } else {
                            // Otherwise it's rows
                            self.rows = value.parse().ok();
                        }
                    }
                    "x" => {
                        self.source_x = value.parse().ok();
                    }
                    "y" => {
                        self.source_y = value.parse().ok();
                    }
                    "w" => {
                        self.source_width = value.parse().ok();
                    }
                    "h" => {
                        self.source_height = value.parse().ok();
                    }
                    "X" => {
                        self.x_offset = value.parse().ok();
                    }
                    "Y" => {
                        self.y_offset = value.parse().ok();
                    }
                    "m" => {
                        self.more_chunks = value == "1";
                    }
                    "d" => {
                        // Delete specification — recorded in `params` above
                        // and resolved after the full key=value list is
                        // parsed, so `d=` may precede its identifying
                        // params (see the post-loop resolution below).
                    }
                    "U" => {
                        // Virtual placement
                        self.is_virtual = value == "1";
                    }
                    "P" => {
                        // Parent image ID for relative positioning
                        self.parent_image_id = value.parse().ok();
                    }
                    "Q" => {
                        // Parent placement ID for relative positioning
                        self.parent_placement_id = value.parse().ok();
                    }
                    "H" => {
                        // Relative X offset in pixels
                        self.relative_x_offset = value.parse().ok();
                    }
                    "V" => {
                        // Relative Y offset in pixels (note: different from v=height).
                        // Parsed unconditionally to match H=; the offset is only *applied*
                        // when a parent (P=) is set (see build_graphic), so storing it
                        // early is harmless and removes the H=/V= asymmetry where V=
                        // was silently dropped unless P= had already been parsed.
                        self.relative_y_offset = value.parse().ok();
                    }
                    "o" => {
                        // Compression format
                        if let Some(c) = value.chars().next() {
                            if let Some(comp) = KittyCompression::from_char(c) {
                                self.compression = comp;
                            }
                        }
                    }
                    "z" => {
                        // z= is overloaded: frame delay for animations, z-index for placements
                        if self.action == KittyAction::Frame {
                            self.frame_delay_ms = value.parse().ok();
                        } else {
                            self.z_index = value.parse().ok();
                        }
                    }
                    "q" => {
                        // Quietness level (0 = reply, 1 = suppress OK, 2 = suppress all)
                        if let Ok(level) = value.parse::<u8>() {
                            self.quietness = level;
                        }
                    }
                    "C" => {
                        // C=1: do not move the cursor after display; C=0 or
                        // omitted keeps the default (cursor advances below
                        // the image) — terminal layer applies the move.
                        self.suppress_cursor_move = value == "1";
                    }
                    _ => {}
                }
            }
        }

        // Resolve delete criteria after the whole key=value list has been
        // parsed: emitters disagree on key order (Herdr sends `d=` before
        // `i=`/`p=`, e.g. `a=d,d=I,i=<id>`), so resolving while scanning
        // would see `image_id`/`placement_id` still unset and silently
        // no-op the delete. `params` accumulates across chunks, so a `d=`
        // seen in an earlier chunk re-resolves once later params arrive.
        if let Some(spec) = self.params.get("d").cloned() {
            self.parse_delete_target(&spec);
        }

        // Decode and accumulate base64 data
        if !data_str.is_empty() {
            // Try STANDARD first (with padding), then NO_PAD if that fails
            // This handles both padded and unpadded base64 (Kitty allows both)
            let decoded =
                base64::Engine::decode(&base64::engine::general_purpose::STANDARD, data_str)
                    .or_else(|_| {
                        base64::Engine::decode(
                            &base64::engine::general_purpose::STANDARD_NO_PAD,
                            data_str,
                        )
                    })
                    .map_err(|e| GraphicsError::Base64Error(e.to_string()))?;
            if self.data_bytes + decoded.len() > MAX_KITTY_PAYLOAD_BYTES {
                // Over the cap: drop the accumulated transmission entirely
                // so a later one starts from zero, not from the overflow.
                self.reset();
                return Err(GraphicsError::KittyError(format!(
                    "payload exceeds {} bytes",
                    MAX_KITTY_PAYLOAD_BYTES
                )));
            }
            self.data_bytes += decoded.len();
            self.data_chunks.push(decoded);
        }

        // Return true if more chunks expected
        Ok(self.more_chunks)
    }

    /// Resolve delete target specification after all parameters are parsed.
    pub(super) fn parse_delete_target(&mut self, value: &str) {
        // Identifying parameters may follow `d=` (as in Herdr's
        // `a=d,d=I,i=<id>` / `a=d,d=i,i=<id>,p=<pid>` ordering).
        if let Some(c) = value.chars().next() {
            self.delete_target = match c {
                'a' | 'A' => Some(KittyDeleteTarget::All),
                'c' | 'C' => Some(KittyDeleteTarget::AtCursor),
                'z' | 'Z' => Some(KittyDeleteTarget::OnScreen),
                'i' | 'I' => self.image_id.map(|iid| match self.placement_id {
                    Some(pid) => KittyDeleteTarget::ByPlacement(iid, Some(pid)),
                    None => KittyDeleteTarget::ById(iid),
                }),
                'p' | 'P' => self
                    .image_id
                    .map(|iid| KittyDeleteTarget::ByPlacement(iid, self.placement_id)),
                'x' | 'X' => self.source_x.map(KittyDeleteTarget::ByColumn),
                'y' | 'Y' => self.source_y.map(KittyDeleteTarget::ByRow),
                _ => None,
            };
        }
    }

    /// Get accumulated data, decompressing if necessary.
    ///
    /// Decompression failures propagate (SEC-116): the old raw-bytes
    /// fallback fed compressed bytes to the pixel decoder on a limit
    /// error — garbage at best, an attack surface at worst.
    pub fn get_data(&self) -> Result<Vec<u8>, GraphicsError> {
        let raw = self.data_chunks.concat();
        if self.compression == KittyCompression::Zlib {
            Self::decompress_zlib(&raw, self.decompression_limit())
        } else {
            Ok(raw)
        }
    }

    /// The zlib output bound for this transmission: with the pixel
    /// geometry known (`f=24`/`f=32` carrying `s=` and `v=`), exactly
    /// width × height × bytes-per-pixel; otherwise the global RGBA worst
    /// case. Either way clamped by [`MAX_KITTY_DECOMPRESSED_BYTES`], so a
    /// lying `s=`/`v=` pair cannot raise the ceiling.
    pub(super) fn decompression_limit(&self) -> usize {
        let bytes_per_pixel = match self.format {
            KittyFormat::Rgb => 3,
            KittyFormat::Rgba => 4,
            KittyFormat::Png => return MAX_KITTY_DECOMPRESSED_BYTES,
        };
        match (self.width, self.height) {
            (Some(width), Some(height)) => (width as usize)
                .saturating_mul(height as usize)
                .saturating_mul(bytes_per_pixel)
                .min(MAX_KITTY_DECOMPRESSED_BYTES),
            _ => MAX_KITTY_DECOMPRESSED_BYTES,
        }
    }

    /// Decompress zlib-compressed data, bounding the output at `limit`
    /// while inflating (SEC-116): `take(limit + 1)` stops the decoder
    /// after one byte past the bound, so an over-limit stream is detected
    /// without ever allocating its full size.
    pub(super) fn decompress_zlib(data: &[u8], limit: usize) -> Result<Vec<u8>, GraphicsError> {
        // Empty input is not a valid zlib stream, but this API treats it as
        // empty output; flate2 versions differ here and callers rely on
        // zero-length payloads not being an error.
        if data.is_empty() {
            return Ok(Vec::new());
        }
        let mut decompressed = Vec::new();
        ZlibDecoder::new(data)
            .take(limit as u64 + 1)
            .read_to_end(&mut decompressed)
            .map_err(|e| GraphicsError::KittyError(format!("Zlib decompression failed: {}", e)))?;
        if decompressed.len() > limit {
            return Err(GraphicsError::KittyError(format!(
                "Decompressed size {} exceeds limit {}",
                decompressed.len(),
                limit
            )));
        }
        Ok(decompressed)
    }

    /// Check if data was compressed
    pub fn is_compressed(&self) -> bool {
        self.compression != KittyCompression::None
    }

    /// Build an ImagePlacement from the parsed Kitty parameters
    pub fn build_placement(&self) -> ImagePlacement {
        let mut placement = ImagePlacement::inline();

        if let Some(cols) = self.columns {
            placement.columns = Some(cols);
            placement.requested_width = ImageDimension::cells(cols as f64);
        }

        if let Some(rows) = self.rows {
            placement.rows = Some(rows);
            placement.requested_height = ImageDimension::cells(rows as f64);
        }

        if let Some(z) = self.z_index {
            placement.z_index = z;
        }

        if let Some(x) = self.source_x {
            placement.source_x = x;
        }
        if let Some(y) = self.source_y {
            placement.source_y = y;
        }
        if let Some(width) = self.source_width {
            placement.source_width = width;
        }
        if let Some(height) = self.source_height {
            placement.source_height = height;
        }
        if let Some(x) = self.x_offset {
            placement.x_offset = x;
        }
        if let Some(y) = self.y_offset {
            placement.y_offset = y;
        }

        placement
    }

    /// Build a TerminalGraphic from parsed data
    pub fn build_graphic(
        &self,
        position: (usize, usize),
        store: &mut GraphicsStore,
    ) -> Result<KittyGraphicResult, GraphicsError> {
        match self.action {
            KittyAction::Delete => self.apply_delete_command(position, store),

            KittyAction::Query => {
                // Query doesn't create a graphic
                Ok(KittyGraphicResult::None)
            }

            KittyAction::Put => self.apply_placement(position, store),

            KittyAction::Transmit | KittyAction::TransmitDisplay => {
                self.apply_transmit_command(position, store)
            }

            KittyAction::Frame => self.apply_frame_command(position, store),

            KittyAction::AnimationControl => self.apply_animation_control(store),
        }
    }
}
