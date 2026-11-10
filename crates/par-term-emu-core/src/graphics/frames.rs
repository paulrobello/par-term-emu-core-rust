//! Kitty animation frames as the protocol defines them.
//!
//! Frame 1 is the image itself (the root frame `a=t`/`a=T` transmitted). `a=f` with `r=N`
//! edits frame N in place, without one, it appends a frame; either way the data is a rectangle
//! composed onto the frame at `x`,`y`, and placements keep their place and size. Placements
//! show the image's current frame (`a=a,c=N`).
//!
//! Reference: <https://sw.kovidgoyal.net/kitty/graphics-protocol/#animation>

use std::sync::Arc;

use super::{AnimationFrame, GraphicsError, GraphicsStore};

/// One `a=f` command: a rectangle of RGBA pixels for a frame of an image.
#[derive(Debug)]
pub(crate) struct FrameEdit {
    /// The frame to edit (`r=`), 1-based; `None` appends a frame.
    pub frame: Option<u32>,
    /// The frame a new frame starts from (`c=`), 1-based.
    pub base: Option<u32>,
    /// Where the rectangle goes in the frame (`x=`, `y=`), in pixels.
    pub x: usize,
    pub y: usize,
    pub width: usize,
    pub height: usize,
    /// RGBA, `width * height * 4` bytes.
    pub pixels: Vec<u8>,
    /// Replace the frame's pixels instead of blending over them (`X=1`, or opaque data).
    pub replace: bool,
    /// A new frame's color without a base frame (`Y=`, 32-bit RGBA).
    pub background: u32,
    /// The frame's gap in milliseconds (`z=`).
    pub gap_ms: Option<u32>,
}

impl GraphicsStore {
    /// Apply an `a=f` command to a stored kitty image. The image must exist, and an edited
    /// frame too.
    pub(crate) fn edit_kitty_frame(
        &mut self,
        image_id: u32,
        edit: FrameEdit,
    ) -> Result<(), GraphicsError> {
        let (width, height, root) = self
            .shared_images
            .get(&image_id)
            .cloned()
            .ok_or_else(|| GraphicsError::KittyError("Image not found".to_string()))?;
        let count = self.kitty_frame_count(image_id);
        let number = match edit.frame {
            Some(number) if number >= 1 && number <= count => number,
            Some(number) => {
                return Err(GraphicsError::KittyError(format!(
                    "Frame {number} does not exist"
                )))
            }
            None => count + 1,
        };
        let whole = edit.x == 0 && edit.y == 0 && edit.width == width && edit.height == height;
        let pixels = if whole && edit.replace {
            edit.pixels
        } else {
            let start = if number <= count {
                self.kitty_frame_pixels(image_id, number, &root)
            } else {
                edit.base
                    .filter(|base| *base >= 1 && *base <= count)
                    .and_then(|base| self.kitty_frame_pixels(image_id, base, &root))
            };
            let mut canvas = match start {
                Some(pixels) => pixels.as_ref().clone(),
                None => edit.background.to_be_bytes().repeat(width * height),
            };
            compose(&mut canvas, width, height, &edit);
            canvas
        };
        let pixels = Arc::new(pixels);

        if number == 1 {
            self.shared_images
                .insert(image_id, (width, height, pixels.clone()));
            if let Some(frame) = self
                .animations
                .get_mut(&image_id)
                .and_then(|animation| animation.frames.get_mut(&1))
            {
                frame.pixels = pixels.clone();
            }
            self.resync_kitty_entry(image_id);
        } else {
            if !self.animations.contains_key(&image_id) {
                // The root frame joins the animation, sharing the image's buffer (already
                // counted against the store budget).
                self.get_or_create_animation(image_id, 100)
                    .add_frame(AnimationFrame::new_shared(1, root, width, height));
            }
            self.add_animation_frame(
                image_id,
                AnimationFrame::new_shared(number, pixels.clone(), width, height),
            );
        }
        if let Some(gap) = edit.gap_ms.filter(|gap| *gap > 0) {
            self.set_kitty_frame_gap(image_id, number, gap);
        }
        let current = self
            .animations
            .get(&image_id)
            .map_or(1, |animation| animation.current_frame);
        if current == number {
            self.show_kitty_pixels(image_id, &pixels);
        }
        Ok(())
    }

    /// `a=a,c=N`: placements of the image show frame N.
    pub(crate) fn set_kitty_current_frame(&mut self, image_id: u32, number: u32) {
        let Some((_, _, root)) = self.shared_images.get(&image_id).cloned() else {
            return;
        };
        let Some(animation) = self.animations.get_mut(&image_id) else {
            // Without an animation the root frame is the only one, and it shows.
            return;
        };
        if animation.current_frame == number || !animation.frames.contains_key(&number) {
            return;
        }
        animation.current_frame = number;
        if animation.frame_start_time.is_some() {
            animation.frame_start_time = Some(std::time::Instant::now());
        }
        if let Some(pixels) = self.kitty_frame_pixels(image_id, number, &root) {
            self.show_kitty_pixels(image_id, &pixels);
        }
    }

    /// `a=a,r=N,z=G` (or `z` on `a=f`): frame N lasts G milliseconds.
    pub(crate) fn set_kitty_frame_gap(&mut self, image_id: u32, number: u32, gap_ms: u32) {
        if let Some(frame) = self
            .animations
            .get_mut(&image_id)
            .and_then(|animation| animation.frames.get_mut(&number))
        {
            frame.delay_ms = gap_ms;
        }
    }

    /// Frames of a stored image, the root frame included.
    fn kitty_frame_count(&self, image_id: u32) -> u32 {
        let extra = self.animations.get(&image_id).map_or(0, |animation| {
            animation
                .frames
                .keys()
                .filter(|number| **number > 1)
                .count()
        });
        1 + extra as u32
    }

    fn kitty_frame_pixels(
        &self,
        image_id: u32,
        number: u32,
        root: &Arc<Vec<u8>>,
    ) -> Option<Arc<Vec<u8>>> {
        if number == 1 {
            return Some(root.clone());
        }
        self.animations
            .get(&image_id)?
            .frames
            .get(&number)
            .map(|frame| frame.pixels.clone())
    }

    /// Every placement of the image, on screen and in the scrollback, shows `pixels`.
    fn show_kitty_pixels(&mut self, image_id: u32, pixels: &Arc<Vec<u8>>) {
        for graphic in self.placements.iter_mut().chain(self.scrollback.iter_mut()) {
            if graphic.kitty_image_id == Some(image_id) && !graphic.is_virtual {
                graphic.pixels = pixels.clone();
            }
        }
    }
}

/// Put the edit's rectangle onto a `width`×`height` RGBA canvas, cut at its edges: copied
/// when the edit replaces, blended over the canvas ("source over") otherwise.
fn compose(canvas: &mut [u8], width: usize, height: usize, edit: &FrameEdit) {
    if edit.x >= width || edit.y >= height {
        return;
    }
    let columns = edit.width.min(width - edit.x);
    let rows = edit.height.min(height - edit.y);
    for row in 0..rows {
        let from = row * edit.width * 4;
        let source = &edit.pixels[from..from + columns * 4];
        let to = ((edit.y + row) * width + edit.x) * 4;
        let target = &mut canvas[to..to + columns * 4];
        if edit.replace {
            target.copy_from_slice(source);
            continue;
        }
        for (dst, src) in target
            .as_chunks_mut::<4>()
            .0
            .iter_mut()
            .zip(source.as_chunks::<4>().0)
        {
            blend(dst, src);
        }
    }
}

/// `src` over `dst`, straight (not premultiplied) alpha.
fn blend(dst: &mut [u8; 4], src: &[u8; 4]) {
    let src_alpha = src[3] as u32;
    if src_alpha == 255 {
        dst.copy_from_slice(src);
        return;
    }
    if src_alpha == 0 {
        return;
    }
    let dst_alpha = dst[3] as u32;
    // out_a = sa + da * (1 - sa), in 0..=255 * 255.
    let out_alpha = src_alpha * 255 + dst_alpha * (255 - src_alpha);
    if out_alpha == 0 {
        dst.copy_from_slice(&[0, 0, 0, 0]);
        return;
    }
    for channel in 0..3 {
        let value = (src[channel] as u32 * src_alpha * 255
            + dst[channel] as u32 * dst_alpha * (255 - src_alpha))
            / out_alpha;
        dst[channel] = value as u8;
    }
    dst[3] = (out_alpha / 255) as u8;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn edit(x: usize, y: usize, width: usize, height: usize, pixels: Vec<u8>) -> FrameEdit {
        FrameEdit {
            frame: Some(1),
            base: None,
            x,
            y,
            width,
            height,
            pixels,
            replace: true,
            background: 0,
            gap_ms: None,
        }
    }

    #[test]
    fn a_rectangle_lands_at_its_offset_and_is_cut_at_the_frame_edge() {
        // 3×2 canvas of zeros; a 2×2 rectangle at (2, 1) keeps only its top-left pixel.
        let mut canvas = vec![0u8; 3 * 2 * 4];
        let pixels: Vec<u8> = (1..=16).collect();
        compose(&mut canvas, 3, 2, &edit(2, 1, 2, 2, pixels));
        let mut expected = vec![0u8; 24];
        expected[20..24].copy_from_slice(&[1, 2, 3, 4]);
        assert_eq!(canvas, expected);
    }

    #[test]
    fn a_rectangle_past_the_frame_changes_nothing() {
        let mut canvas = vec![7u8; 2 * 2 * 4];
        compose(&mut canvas, 2, 2, &edit(2, 0, 1, 1, vec![1, 2, 3, 4]));
        assert_eq!(canvas, vec![7u8; 16]);
    }

    #[test]
    fn blending_mixes_by_alpha_and_replacing_copies() {
        let mut dst = [0, 0, 200, 255];
        blend(&mut dst, &[200, 0, 0, 128]);
        assert_eq!(dst, [100, 0, 99, 255]);

        let mut dst = [10, 20, 30, 255];
        blend(&mut dst, &[1, 2, 3, 0]);
        assert_eq!(
            dst,
            [10, 20, 30, 255],
            "a transparent pixel leaves the frame"
        );

        let mut canvas = vec![9u8; 4];
        let mut replace = edit(0, 0, 1, 1, vec![1, 2, 3, 0]);
        compose(&mut canvas, 1, 1, &replace);
        assert_eq!(
            canvas,
            vec![1, 2, 3, 0],
            "X=1 copies even transparent pixels"
        );
        replace.replace = false;
        let mut canvas = vec![9u8; 4];
        compose(&mut canvas, 1, 1, &replace);
        assert_eq!(canvas, vec![9u8; 4]);
    }
}
