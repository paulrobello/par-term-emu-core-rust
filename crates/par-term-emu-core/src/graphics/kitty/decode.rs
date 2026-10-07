//! Payload decoding: transmission media (direct, file, temp file, shared
//! memory) with their filesystem safety gates, and pixel-format decoding.

use super::*;

impl KittyParser {
    /// Load image data per transmission medium and decode it to raw RGBA
    /// pixels — the payload-resolution step shared by the transmit and
    /// frame handlers. `raw_data` is the already-concatenated (and, if
    /// applicable, decompressed) payload from [`Self::get_data`];
    /// `shared_mem_error` is the caller's error message for the
    /// (unsupported) `SharedMem` medium, since Transmit and Frame each use
    /// distinct wording there.
    pub(super) fn decode_payload(
        &self,
        raw_data: Vec<u8>,
        shared_mem_error: &str,
    ) -> Result<(usize, usize, Vec<u8>), GraphicsError> {
        // Load image data based on transmission medium. For file media the
        // load also returns the `t=t` cleanup path: deletion is deferred
        // until the payload decodes, so a non-image temp file is never
        // destroyed (SEC-101).
        let (image_data, delete_after_decode) = match self.medium {
            KittyMedium::File | KittyMedium::TempFile => {
                // For file transmission, raw_data is a file path (not base64-encoded)
                self.load_file_data(&raw_data)?
            }
            KittyMedium::Direct => {
                // For direct transmission, use data as-is
                (raw_data, None)
            }
            KittyMedium::SharedMem => {
                return Err(GraphicsError::KittyError(shared_mem_error.to_string()));
            }
        };

        let decoded = self.decode_pixels(&image_data);

        // Delete the t=t temp file only once the payload proved decodable.
        // Skipped when `retain_temp_files` is set (mux daemon terminals):
        // client mirrors re-read the same file from forwarded bytes — the
        // rendering client deletes it.
        if decoded.is_ok() {
            if let Some(pending) = delete_after_decode {
                delete_if_same_file(&pending);
            }
        }

        decoded
    }

    /// Load image data from a file medium (`t=f`/`t=t`) with security
    /// gating (SEC-101/SEC-102/SEC-103).
    ///
    /// Returns the file bytes plus, for a non-retained `t=t` read, the
    /// file the caller must delete **after** the payload decodes.
    pub(super) fn load_file_data(
        &self,
        path_data: &[u8],
    ) -> Result<(Vec<u8>, Option<PendingDelete>), GraphicsError> {
        // Decode path from UTF-8 bytes (NOT base64-encoded for file transmission)
        let path_str = String::from_utf8(path_data.to_vec())
            .map_err(|e| GraphicsError::KittyError(format!("Invalid UTF-8 in file path: {}", e)))?;

        let path = Path::new(&path_str);

        // Security validations

        // 1. Reject paths containing a real parent-directory component.
        //    Component-wise, not substring: "my..notes.png" has no `..`
        //    component and must stay readable, while "a/../b" must not.
        if path.components().any(|c| c == Component::ParentDir) {
            return Err(GraphicsError::KittyError(
                "Directory traversal not allowed".to_string(),
            ));
        }

        // 2. Mode gate (SEC-101/SEC-102): file media is opt-in per medium,
        //    and is checked before touching the filesystem so a probe
        //    leaks no existence information.
        match self.medium {
            KittyMedium::File => {
                if self.allow_file_media != FileMediaMode::All {
                    return Err(GraphicsError::KittyError(
                        "File medium (t=f) requires allow_file_media=\"all\"".to_string(),
                    ));
                }
            }
            KittyMedium::TempFile if self.allow_file_media == FileMediaMode::Off => {
                return Err(GraphicsError::KittyError(
                    "Temp-file medium (t=t) is disabled (allow_file_media=\"off\")".to_string(),
                ));
            }
            _ => {}
        }

        // 3. t=t path gate (SEC-101): the canonicalized path must sit under
        //    an allowed temp root AND carry the spec's marker in its
        //    filename — kitty's own rule for the temp-file medium. Refuse
        //    without touching the file otherwise.
        let canonical = if self.medium == KittyMedium::TempFile {
            let canonical = path.canonicalize().map_err(|e| {
                GraphicsError::KittyError(format!("Cannot resolve temp file path: {}", e))
            })?;
            check_temp_file_gate(&canonical)?;
            Some(canonical)
        } else {
            None
        };

        // 4. Open without following a final symlink (SEC-103) and validate
        //    the handle itself, so the file we read is the file we checked.
        //    `t=t` opens the canonical path that passed the gate (SEC-130);
        //    `t=f` has no containment check, so it opens the path as given.
        let open_path = canonical.as_deref().unwrap_or(path);
        let mut file = open_no_follow(open_path).map_err(|e| {
            // The directory check comes first: Windows fails opening a
            // directory with ERROR_PATH_NOT_FOUND (io kind NotFound), so a
            // kind-ordered check would report an existing directory as
            // "File not found".
            if path.is_dir() {
                GraphicsError::KittyError(format!("Path is not a file: {}", path_str))
            } else if e.kind() == std::io::ErrorKind::NotFound {
                GraphicsError::KittyError(format!("File not found: {}", path_str))
            } else {
                GraphicsError::KittyError(format!("Cannot open file: {}", e))
            }
        })?;

        let metadata = file
            .metadata()
            .map_err(|e| GraphicsError::KittyError(format!("Cannot read file metadata: {}", e)))?;

        if !metadata.is_file() {
            return Err(GraphicsError::KittyError(format!(
                "Path is not a file: {}",
                path_str
            )));
        }

        // 4b. t=t: re-run the gate on the path the kernel reports for the
        //     open handle (SEC-130). A parent directory swapped for a
        //     symlink between canonicalize and open escapes O_NOFOLLOW,
        //     which guards only the final component; this catches it.
        if canonical.is_some() {
            if let Some(resolved) = opened_path(&file) {
                let resolved = resolved.map_err(|e| {
                    GraphicsError::KittyError(format!("Cannot resolve opened temp file: {}", e))
                })?;
                check_temp_file_gate(&resolved)?;
            }
        }

        // 5. Check file size (limit to 100MB for safety)
        /// cap: Bytes read from one kitty file medium named by an escape payload.
        const MAX_FILE_SIZE: u64 = 100 * 1024 * 1024; // 100MB
        if metadata.len() > MAX_FILE_SIZE {
            return Err(GraphicsError::KittyError(format!(
                "File too large: {} bytes (max {})",
                metadata.len(),
                MAX_FILE_SIZE
            )));
        }

        // 6. Read from the validated handle
        let mut file_data = Vec::with_capacity(metadata.len() as usize);
        file.read_to_end(&mut file_data)
            .map_err(|e| GraphicsError::KittyError(format!("Cannot read file: {}", e)))?;

        // Pending delete: only a non-retained t=t read deletes, and only
        // after decode succeeds (the caller's job). It is pinned to the
        // inode read here, so a file swapped in afterwards survives.
        let pending_delete = match canonical {
            Some(canonical) if !self.retain_temp_files => {
                #[cfg(unix)]
                let dev_ino = {
                    use std::os::unix::fs::MetadataExt;
                    (metadata.dev(), metadata.ino())
                };
                Some(PendingDelete {
                    path: canonical,
                    #[cfg(unix)]
                    dev_ino,
                })
            }
            _ => None,
        };

        Ok((file_data, pending_delete))
    }

    /// Decode pixels based on format
    pub(super) fn decode_pixels(
        &self,
        data: &[u8],
    ) -> Result<(usize, usize, Vec<u8>), GraphicsError> {
        match self.format {
            KittyFormat::Png => {
                // Decode PNG via a size-limited reader so the decoder refuses
                // to allocate before hitting MAX_IMAGE_DIMENSION, guarding
                // against decompression-bomb PNGs (small input, huge output).
                let mut reader = image::ImageReader::new(std::io::Cursor::new(data))
                    .with_guessed_format()
                    .map_err(|e| GraphicsError::ImageError(e.to_string()))?;
                let mut limits = image::Limits::default();
                limits.max_image_width = Some(MAX_IMAGE_DIMENSION as u32);
                limits.max_image_height = Some(MAX_IMAGE_DIMENSION as u32);
                reader.limits(limits);
                let img = reader
                    .decode()
                    .map_err(|e| GraphicsError::ImageError(e.to_string()))?;
                let rgba = img.to_rgba8();
                let width = rgba.width() as usize;
                let height = rgba.height() as usize;
                // Also cap the total pixel product: two dimensions can each
                // pass MAX_IMAGE_DIMENSION individually yet still multiply
                // out to a huge RGBA buffer.
                let pixels = width.checked_mul(height).ok_or_else(|| {
                    GraphicsError::KittyError("Image dimensions overflow usize".to_string())
                })?;
                if pixels > MAX_IMAGE_PIXELS {
                    return Err(GraphicsError::ImageTooLarge(pixels, MAX_IMAGE_PIXELS));
                }
                Ok((width, height, rgba.into_raw()))
            }

            KittyFormat::Rgba => {
                // Raw RGBA data
                let width = self.width.ok_or_else(|| {
                    GraphicsError::KittyError("Width required for raw format".to_string())
                })? as usize;
                let height = self.height.ok_or_else(|| {
                    GraphicsError::KittyError("Height required for raw format".to_string())
                })? as usize;
                // checked_mul: width/height are attacker-controlled u32 values; without
                // this, `width * height * 4` can wrap usize and bypass the size check,
                // yielding a graphic with huge dims over a tiny buffer (OOB/panic DoS).
                let expected = width
                    .checked_mul(height)
                    .and_then(|px| px.checked_mul(4))
                    .ok_or_else(|| {
                        GraphicsError::KittyError("Image dimensions overflow usize".to_string())
                    })?;
                if data.len() != expected {
                    return Err(GraphicsError::KittyError(format!(
                        "Data size mismatch: got {}, expected {}",
                        data.len(),
                        expected
                    )));
                }
                Ok((width, height, data.to_vec()))
            }

            KittyFormat::Rgb => {
                // Raw RGB data - convert to RGBA
                let width = self.width.ok_or_else(|| {
                    GraphicsError::KittyError("Width required for raw format".to_string())
                })? as usize;
                let height = self.height.ok_or_else(|| {
                    GraphicsError::KittyError("Height required for raw format".to_string())
                })? as usize;
                // checked_mul throughout (see Rgba): attacker-controlled dimensions.
                let pixels = width.checked_mul(height).ok_or_else(|| {
                    GraphicsError::KittyError("Image dimensions overflow usize".to_string())
                })?;
                let expected = pixels.checked_mul(3).ok_or_else(|| {
                    GraphicsError::KittyError("Image dimensions overflow usize".to_string())
                })?;
                if data.len() != expected {
                    return Err(GraphicsError::KittyError(format!(
                        "Data size mismatch: got {}, expected {}",
                        data.len(),
                        expected
                    )));
                }

                // Convert RGB to RGBA (capacity validated non-overflowing).
                let mut rgba = Vec::with_capacity(pixels.checked_mul(4).ok_or_else(|| {
                    GraphicsError::KittyError("Image dimensions overflow usize".to_string())
                })?);
                for chunk in data.chunks(3) {
                    rgba.push(chunk[0]);
                    rgba.push(chunk[1]);
                    rgba.push(chunk[2]);
                    rgba.push(255); // Alpha
                }
                Ok((width, height, rgba))
            }
        }
    }
}
