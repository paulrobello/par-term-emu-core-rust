/**
 * Inline-image frame rendering for the web terminal.
 *
 * A ServerMessage::InlineImage is the render half of the inline-image
 * path: the xterm ImageAddon has no programmatic ingest, so the frame is
 * re-materialized as the iTerm2 inline-image sequence and written to the
 * terminal — the addon decodes and draws it through the same machinery a
 * locally decoded image uses. Only "rgba" frames render; other formats are
 * logged and skipped.
 */

import type { InlineImage } from '@/lib/proto/terminal_pb';

/** Only format tag this renderer decodes: the wire carries store pixels. */
export const INLINE_IMAGE_FORMAT = 'rgba';

/**
 * Build the iTerm2 `File=` sequence for a frame, or `null` for a frame
 * this renderer does not render (wrong format, zero pixel extent, or a
 * payload shorter than the declared pixels).
 *
 * The sequence has no cell position: the addon draws at the terminal's
 * current cursor. In the live stream a frame arrives immediately after the
 * image's own bytes, so the cursor sits right below the image — the same
 * placement a locally decoded image gets.
 */
export function inlineImageSequence(frame: InlineImage): string | null {
  const expected = frame.width * frame.height * 4;
  if (frame.format !== INLINE_IMAGE_FORMAT) {
    console.warn(
      `Inline image ${frame.imageId}: format '${frame.format}' is not ` +
        `${INLINE_IMAGE_FORMAT}; not rendered`,
    );
    return null;
  }
  if (frame.width === 0 || frame.height === 0) {
    console.warn(`Inline image ${frame.imageId}: zero pixel extent; not rendered`);
    return null;
  }
  if (frame.data.length < expected) {
    console.warn(
      `Inline image ${frame.imageId}: ${frame.data.length} bytes for ` +
        `${expected}; not rendered`,
    );
    return null;
  }
  const canvas = document.createElement('canvas');
  canvas.width = frame.width;
  canvas.height = frame.height;
  const ctx = canvas.getContext('2d');
  if (!ctx) {
    console.warn(`Inline image ${frame.imageId}: no 2d context; not rendered`);
    return null;
  }
  const imageData = ctx.createImageData(frame.width, frame.height);
  imageData.data.set(frame.data.subarray(0, expected));
  ctx.putImageData(imageData, 0, 0);
  return canvasToItermSequence(canvas);
}

/**
 * Encode a canvas as the iTerm2 inline-image sequence — ESC ] 1337 ; File=
 * name=...;inline=1;width=Wpx;height=Hpx:<base64 png> BEL — which the
 * ImageAddon's iipSupport path decodes exactly like a locally placed image.
 */
export function canvasToItermSequence(canvas: HTMLCanvasElement): string {
  const base64 = canvas
    .toDataURL('image/png')
    .slice('data:image/png;base64,'.length);
  return (
    `\x1b]1337;File=name=${btoa('inline.png')};inline=1;` +
    `width=${canvas.width}px;height=${canvas.height}px:${base64}\x07`
  );
}
