//! Kitty command dispatch: delete, placement, transmit, frame, and
//! animation-control commands applied to the graphics store.

use super::*;
use crate::graphics::FrameEdit;

impl KittyParser {
    /// Handle `a=d` (delete): resolve the delete target against the
    /// graphics store and remove matching placements/images.
    pub(super) fn apply_delete_command(
        &self,
        position: (usize, usize),
        store: &mut GraphicsStore,
    ) -> Result<KittyGraphicResult, GraphicsError> {
        if let Some(target) = &self.delete_target {
            match target {
                KittyDeleteTarget::All => store.clear(),
                KittyDeleteTarget::ById(id) => {
                    store.delete_kitty_graphics(Some(*id), None);
                }
                KittyDeleteTarget::ByPlacement(iid, pid) => {
                    store.delete_kitty_graphics(Some(*iid), *pid);
                }
                KittyDeleteTarget::AtCursor => {
                    let (cursor_col, cursor_row) = position;
                    store
                        .placements
                        .retain(|g| g.position != (cursor_col, cursor_row));
                }
                KittyDeleteTarget::InCell => {
                    // Same as AtCursor in our context since we use the cursor position
                    let (cursor_col, cursor_row) = position;
                    store
                        .placements
                        .retain(|g| g.position != (cursor_col, cursor_row));
                }
                KittyDeleteTarget::OnScreen => {
                    // Remove all visible placements but preserve shared images
                    store.placements.clear();
                }
                KittyDeleteTarget::ByColumn(col) => {
                    let target_col = *col as usize;
                    store.placements.retain(|g| {
                        let start_col = g.position.0;
                        let cell_width = g.cell_dimensions.map(|(w, _)| w as usize).unwrap_or(1);
                        let end_col = start_col + g.width.div_ceil(cell_width);
                        target_col < start_col || target_col >= end_col
                    });
                }
                KittyDeleteTarget::ByRow(row) => {
                    let target_row = *row as usize;
                    store.placements.retain(|g| {
                        let start_row = g.position.1;
                        let cell_height = g.cell_dimensions.map(|(_, h)| h as usize).unwrap_or(2);
                        let end_row = start_row + g.height.div_ceil(cell_height);
                        target_row < start_row || target_row >= end_row
                    });
                }
            }
        }
        Ok(KittyGraphicResult::None)
    }

    /// Handle `a=p` (put): display a previously transmitted image, or
    /// create a virtual (Unicode-placeholder) placement when `U=1`.
    pub(super) fn apply_placement(
        &self,
        position: (usize, usize),
        store: &mut GraphicsStore,
    ) -> Result<KittyGraphicResult, GraphicsError> {
        // Display previously transmitted image or create virtual placement
        let image_id = self.image_id.unwrap_or(0);

        // If U=1, create a virtual placement
        if self.is_virtual {
            let cols = self.columns.unwrap_or(1) as usize;
            let rows = self.rows.unwrap_or(1) as usize;
            let placement_id = self.placement_id.unwrap_or(0);

            // Create virtual placement without image data
            let mut graphic = TerminalGraphic::new(
                next_graphic_id(),
                GraphicProtocol::Kitty,
                position,
                cols,
                rows,
                vec![], // Virtual placements don't need pixel data
            );
            graphic.kitty_image_id = Some(image_id);
            graphic.kitty_placement_id = Some(placement_id);
            graphic.is_virtual = true;
            store.add_virtual_placement(graphic);

            // Return virtual placement info for placeholder insertion
            return Ok(KittyGraphicResult::VirtualPlacement {
                image_id,
                placement_id,
                position,
                cols,
                rows,
            });
        }

        // Regular placement
        if let Some((width, height, pixels)) = store.get_kitty_image(image_id) {
            let mut graphic = TerminalGraphic::with_shared_pixels(
                next_graphic_id(),
                GraphicProtocol::Kitty,
                position,
                width,
                height,
                pixels,
            );
            graphic.kitty_image_id = Some(image_id);
            graphic.kitty_placement_id = self.placement_id;
            graphic.placement = self.build_placement();

            // Handle relative positioning
            if let Some(parent_img_id) = self.parent_image_id {
                graphic.parent_image_id = Some(parent_img_id);
                graphic.parent_placement_id = self.parent_placement_id;
                graphic.relative_x_offset = self.relative_x_offset.unwrap_or(0);
                graphic.relative_y_offset = self.relative_y_offset.unwrap_or(0);
            }

            return Ok(KittyGraphicResult::Graphic(graphic));
        }
        Err(GraphicsError::KittyError("Image not found".to_string()))
    }

    /// Handle `a=t`/`a=T` (transmit / transmit+display): decode the
    /// payload per the transmission medium/format, cache it if an image id
    /// was given, and build a graphic when the action is `TransmitDisplay`.
    pub(super) fn apply_transmit_command(
        &self,
        position: (usize, usize),
        store: &mut GraphicsStore,
    ) -> Result<KittyGraphicResult, GraphicsError> {
        let raw_data = self.get_data()?;
        if raw_data.is_empty() {
            return Err(GraphicsError::KittyError("No image data".to_string()));
        }

        let compressed = self.is_compressed();

        let (width, height, pixels) =
            self.decode_payload(raw_data, "Shared memory transmission not supported")?;
        let pixels = std::sync::Arc::new(pixels);

        // Store for reuse if image_id is specified
        if let Some(image_id) = self.image_id {
            store.store_kitty_image_shared(image_id, width, height, pixels.clone());
        }

        // Create graphic if TransmitDisplay, or virtual placement if U=1
        if self.action == KittyAction::TransmitDisplay {
            if self.is_virtual {
                let cols = self.columns.unwrap_or(1) as usize;
                let rows = self.rows.unwrap_or(1) as usize;
                let image_id = self.image_id.unwrap_or(0);
                let placement_id = self.placement_id.unwrap_or(0);

                // Create virtual placement
                let mut graphic = TerminalGraphic::new(
                    next_graphic_id(),
                    GraphicProtocol::Kitty,
                    position,
                    cols,
                    rows,
                    vec![], // Virtual placements don't need pixel data
                );
                graphic.kitty_image_id = Some(image_id);
                graphic.kitty_placement_id = Some(placement_id);
                graphic.is_virtual = true;
                graphic.was_compressed = compressed;
                store.add_virtual_placement(graphic);

                // Return virtual placement info for placeholder insertion
                Ok(KittyGraphicResult::VirtualPlacement {
                    image_id,
                    placement_id,
                    position,
                    cols,
                    rows,
                })
            } else {
                let mut graphic = TerminalGraphic::with_shared_pixels(
                    next_graphic_id(),
                    GraphicProtocol::Kitty,
                    position,
                    width,
                    height,
                    pixels,
                );
                graphic.kitty_image_id = self.image_id;
                graphic.kitty_placement_id = self.placement_id;
                graphic.was_compressed = compressed;
                graphic.placement = self.build_placement();

                // Handle relative positioning
                if let Some(parent_img_id) = self.parent_image_id {
                    graphic.parent_image_id = Some(parent_img_id);
                    graphic.parent_placement_id = self.parent_placement_id;
                    graphic.relative_x_offset = self.relative_x_offset.unwrap_or(0);
                    graphic.relative_y_offset = self.relative_y_offset.unwrap_or(0);
                }

                Ok(KittyGraphicResult::Graphic(graphic))
            }
        } else {
            // Transmit only, no display
            Ok(KittyGraphicResult::None)
        }
    }

    /// Handle `a=f` (animation frame). As kitty does, the data is a rectangle
    /// for a frame of an existing image: `r=N` edits frame N in place (frame 1 is the image
    /// itself), no `r=` appends a frame. Placements keep their place and size and the cursor
    /// stays; they show the edited pixels when it is the image's current frame.
    pub(super) fn apply_frame_command(
        &self,
        store: &mut GraphicsStore,
    ) -> Result<KittyGraphicResult, GraphicsError> {
        let raw_data = self.get_data()?;
        if raw_data.is_empty() {
            return Err(GraphicsError::KittyError("No frame data".to_string()));
        }

        let image_id = self
            .image_id
            .ok_or_else(|| GraphicsError::KittyError("Frame requires image ID".to_string()))?;

        let (width, height, pixels) =
            self.decode_payload(raw_data, "Shared memory not supported for frames")?;

        store.edit_kitty_frame(
            image_id,
            FrameEdit {
                frame: self.frame_number,
                base: self.base_frame,
                x: self.source_x.unwrap_or(0) as usize,
                y: self.source_y.unwrap_or(0) as usize,
                width,
                height,
                pixels,
                // RGB data is opaque: blending it over the frame replaces the pixels.
                replace: self.frame_composition == Some(CompositionMode::Overwrite)
                    || self.format == KittyFormat::Rgb,
                background: self.background_color.unwrap_or(0),
                gap_ms: self.frame_delay_ms,
            },
        )?;
        Ok(KittyGraphicResult::None)
    }

    /// Handle `a=a` (animation control): update loop count and/or playback
    /// state for an existing animated image.
    pub(super) fn apply_animation_control(
        &self,
        store: &mut GraphicsStore,
    ) -> Result<KittyGraphicResult, GraphicsError> {
        // Control animation playback
        let image_id = self.image_id.ok_or_else(|| {
            GraphicsError::KittyError("Animation control requires image ID".to_string())
        })?;

        // Handle num_plays (v= parameter) for setting loop count
        // Per Kitty spec: v=0 ignored, v=1 infinite, v=N means play N times total
        // We store loop_count as (N-1) so animation stops after (N-1) additional loops
        if let Some(num_plays) = self.num_plays {
            if num_plays > 0 {
                let loop_count = if num_plays == 1 {
                    0 // v=1 means infinite looping
                } else {
                    num_plays - 1 // Store N-1 to get N total plays
                };
                debug_info!(
                    "KITTY",
                    "Setting loop count for image_id={}: num_plays={}, loop_count={}",
                    image_id,
                    num_plays,
                    loop_count
                );
                store.set_animation_loops(image_id, loop_count);
            }
        }

        // `r=` with `z=` sets that frame's gap, `c=` shows a frame.
        if let (Some(number), Some(gap)) = (self.frame_number, self.frame_delay_ms) {
            if gap > 0 {
                store.set_kitty_frame_gap(image_id, number, gap);
            }
        }
        if let Some(number) = self.current_frame.filter(|number| *number > 0) {
            store.set_kitty_current_frame(image_id, number);
        }

        // Handle state control (s= parameter)
        if let Some(control) = self.animation_control {
            debug_info!(
                "KITTY",
                "Applying animation control: image_id={}, control={:?}",
                image_id,
                control
            );
            store.control_animation(image_id, control);
        } else {
            debug_log!(
                "KITTY",
                "Animation control command received but no control parsed (image_id={})",
                image_id
            );
        }

        Ok(KittyGraphicResult::None)
    }
}
