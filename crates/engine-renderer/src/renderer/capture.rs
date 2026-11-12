//! One-shot clean-frame capture of the surface.
//!
//! The copy is recorded in `pass_postfx` right after the tonemap / post chain
//! and before any UI pass, so the captured frame holds
//! the tonemapped (post-processed) scene only (no menu/HUD). Readback follows the same
//! submit → `map_async` → next-frame poll pattern as the luminance and
//! occlusion readbacks, including the deferred-submit path
//! (`finish_deferred_surface_submit`), so it is safe with the pipelined
//! render thread.

use std::sync::mpsc;

use super::Renderer;

/// A captured frame: tightly packed RGBA8, srgb-encoded bytes as presented.
/// BGR(A) surfaces are swizzled to RGBA, 256-aligned rows are unpacked.
pub struct CapturedFrame {
    pub rgba: Vec<u8>,
    pub width: u32,
    pub height: u32,
}

/// `bytes_per_row` for `copy_texture_to_buffer` must be 256-aligned.
pub(crate) fn padded_bytes_per_row(width: u32) -> u32 {
    (width * 4).next_multiple_of(256)
}

/// Unpack 256-aligned rows into a tightly packed RGBA8 buffer, swizzling
/// BGRA→RGBA when the surface format was BGR(A).
pub(crate) fn unpack_capture(
    padded: &[u8],
    width: u32,
    height: u32,
    swizzle_bgra: bool,
) -> Vec<u8> {
    let w = width as usize;
    let stride = padded_bytes_per_row(width) as usize;
    debug_assert!(padded.len() >= stride * height as usize);
    let mut out = Vec::with_capacity(w * height as usize * 4);
    for row in padded.chunks(stride).take(height as usize) {
        let src = &row[..w * 4];
        if swizzle_bgra {
            for px in src.as_chunks::<4>().0 {
                out.extend_from_slice(&[px[2], px[1], px[0], px[3]]);
            }
        } else {
            out.extend_from_slice(src);
        }
    }
    out
}

enum CapturePhase {
    /// Copy recorded on the frame encoder; waiting for the owning submit.
    Copied {
        buffer: wgpu::Buffer,
        width: u32,
        height: u32,
        swizzle: bool,
    },
    /// `map_async` requested after submit; waiting for the GPU.
    Mapping {
        buffer: wgpu::Buffer,
        width: u32,
        height: u32,
        swizzle: bool,
        rx: mpsc::Receiver<Result<(), wgpu::BufferAsyncError>>,
    },
}

pub(crate) struct CaptureState {
    /// Capture requested by the game, not yet recorded on an encoder.
    pending: bool,
    phase: Option<CapturePhase>,
    /// Finished frame waiting for `capture_take`.
    captured: Option<CapturedFrame>,
}

impl CaptureState {
    pub(crate) const fn new() -> Self {
        Self {
            pending: false,
            phase: None,
            captured: None,
        }
    }

    fn busy(&self) -> bool {
        self.pending || self.phase.is_some()
    }

    /// True when a capture is queued and not yet recorded on an encoder.
    pub(crate) fn wants_capture(&self) -> bool {
        self.pending
    }
}

impl Renderer {
    /// Queue a one-shot capture of the next rendered frame. Returns `true` if
    /// queued; `false` when there is no surface or a capture is already in
    /// flight.
    pub fn capture_request(&mut self) -> bool {
        if self.surface_config.is_none() || self.capture.busy() {
            return false;
        }
        self.capture.pending = true;
        true
    }

    /// Hand out the captured frame. `1` = written to `out` (dims in `w`/`h`),
    /// `0` = nothing ready yet, `-1` = `out` too small (frame is kept, `w`/`h`
    /// receive the required dims).
    pub fn capture_take(&mut self, out: &mut [u8], w: &mut u32, h: &mut u32) -> i32 {
        let Some(frame) = self.capture.captured.as_ref() else {
            return 0;
        };
        *w = frame.width;
        *h = frame.height;
        if out.len() < frame.rgba.len() {
            return -1;
        }
        out.copy_from_slice(&frame.rgba);
        self.capture.captured = None;
        1
    }

    /// Record the surface copy for a pending capture. Called from
    /// `pass_postfx` after the tonemap / post chain, before the UI pass.
    pub(crate) fn encode_capture_copy(&mut self, encoder: &mut wgpu::CommandEncoder) {
        if !self.capture.pending {
            return;
        }
        // The swapchain texture is stashed by the render entry point of this
        // frame; without it (headless/offscreen frame) the request stays queued.
        let Some(texture) = self.frame_surface_texture.take() else {
            return;
        };
        let Some(cfg) = self.surface_config.as_ref() else {
            self.capture.pending = false;
            return;
        };
        let (width, height, format) = (cfg.width, cfg.height, cfg.format);
        let bytes_per_row = padded_bytes_per_row(width);
        // One-shot: a fresh staging buffer per request, sized for this surface,
        // so resizes need no explicit invalidation.
        let buffer = self.gpu.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("capture staging"),
            size: bytes_per_row as u64 * height as u64,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(bytes_per_row),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d {
                width,
                height,
                depth_or_array_layers: 1,
            },
        );
        let swizzle = matches!(
            format,
            wgpu::TextureFormat::Bgra8Unorm | wgpu::TextureFormat::Bgra8UnormSrgb
        );
        self.capture.pending = false;
        self.capture.phase = Some(CapturePhase::Copied {
            buffer,
            width,
            height,
            swizzle,
        });
    }

    /// Request the readback map. Must run AFTER the encoder holding the copy
    /// was submitted (immediate submit in `finish_surface_encoder` or the
    /// deferred pipelined submit in `finish_deferred_surface_submit`).
    pub(crate) fn queue_capture_readback_after_submit(&mut self) {
        let Some(CapturePhase::Copied {
            buffer,
            width,
            height,
            swizzle,
        }) = self.capture.phase.take()
        else {
            return;
        };
        let (tx, rx) = mpsc::channel();
        buffer
            .slice(..)
            .map_async(wgpu::MapMode::Read, move |result| {
                let _ = tx.send(result);
            });
        self.capture.phase = Some(CapturePhase::Mapping {
            buffer,
            width,
            height,
            swizzle,
            rx,
        });
    }

    /// Poll the pending map; on completion unpack and store the frame.
    pub(crate) fn process_capture_readback(&mut self) {
        let Some(CapturePhase::Mapping {
            buffer,
            width,
            height,
            swizzle,
            rx,
        }) = self.capture.phase.take()
        else {
            return;
        };
        let _ = self.gpu.device.poll(wgpu::PollType::Poll);
        match rx.try_recv() {
            Ok(Ok(())) => {
                let slice = buffer.slice(..);
                let data = match slice.get_mapped_range() {
                    Ok(d) => d,
                    Err(e) => {
                        eprintln!("[gpu] capture map range error: {e:?}");
                        buffer.unmap();
                        return;
                    }
                };
                let rgba = unpack_capture(&data, width, height, swizzle);
                drop(data);
                buffer.unmap();
                self.capture.captured = Some(CapturedFrame {
                    rgba,
                    width,
                    height,
                });
            }
            Ok(Err(e)) => {
                eprintln!("[gpu] capture map error: {e:?}");
                buffer.unmap();
            }
            Err(mpsc::TryRecvError::Empty) => {
                // Still pending — keep waiting, checked again next frame.
                self.capture.phase = Some(CapturePhase::Mapping {
                    buffer,
                    width,
                    height,
                    swizzle,
                    rx,
                });
            }
            Err(mpsc::TryRecvError::Disconnected) => {
                // Callback channel dropped; the buffer may still be mapped.
                buffer.unmap();
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn padded_row_is_256_aligned() {
        assert_eq!(padded_bytes_per_row(1), 256);
        assert_eq!(padded_bytes_per_row(64), 256); // 64 px = 256 B
        assert_eq!(padded_bytes_per_row(65), 512);
        assert_eq!(padded_bytes_per_row(1920), 7680); // 7680 = 30*256
    }

    #[test]
    fn unpack_rgba_strips_padding() {
        // 2 px wide → padded stride 256, row data 8 bytes each.
        let mut padded = vec![0u8; 256 * 2];
        padded[..8].copy_from_slice(&[1, 2, 3, 4, 5, 6, 7, 8]);
        padded[256..264].copy_from_slice(&[9, 10, 11, 12, 13, 14, 15, 16]);
        let out = unpack_capture(&padded, 2, 2, false);
        assert_eq!(
            out,
            vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16]
        );
    }

    #[test]
    fn unpack_swizzles_bgra_to_rgba() {
        // 1 px wide → stride 256, single BGRA pixel.
        let mut padded = vec![0u8; 256];
        padded[..4].copy_from_slice(&[10, 20, 30, 255]); // B,G,R,A
        let out = unpack_capture(&padded, 1, 1, true);
        assert_eq!(out, vec![30, 20, 10, 255]); // R,G,B,A
    }
}
