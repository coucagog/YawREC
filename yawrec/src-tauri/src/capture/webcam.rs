// ============================================================
// YAWREC · capture/webcam.rs (D4)
// Capture webcam via `nokhwa` (MediaFoundation sur Windows).
//
// Architecture :
//   - Le worker webcam (lancé par commands.rs) possède la Camera nokhwa,
//     boucle sur frame() pour récupérer du RGB, le convertit/redimensionne
//     vers BGRA aux dimensions PiP, et écrit dans Arc<Mutex<Option<Frame>>>.
//   - Le worker vidéo (commands.rs) lit ce buffer, blitte sur la frame
//     écran avant de pousser à l'encoder.
//   - La Camera nokhwa est Send mais !Sync : ouverte et possédée dans le
//     thread worker, jamais déplacée.
// ============================================================

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::capture::Frame;
use crate::commands::DeviceInfo;

#[derive(Debug, thiserror::Error)]
pub enum WebcamError {
    #[error("Énumération : {0}")]
    Enumeration(String),
    #[error("Périphérique non trouvé : {0}")]
    NotFound(String),
    #[error("Format non supporté : {0}")]
    Format(String),
    #[error("Capture : {0}")]
    Capture(String),
    #[error("Plateforme non supportée")]
    Unsupported,
}

// ============================================================
// PiP — dimensions de la vignette webcam
// ============================================================
// 400 × 225 = 16:9, visible sans envahir l'écran à 1080p (≈ 9 % de surface).
// La position est gérée côté composite (commands.rs::composite_pip) :
// bas-droite avec MARGIN px depuis chaque bord.
pub const PIP_WIDTH:  u32 = 400;
pub const PIP_HEIGHT: u32 = 225;
pub const PIP_MARGIN: u32 = 24;

// ============================================================
// API publique
// ============================================================

pub fn list_devices() -> Result<Vec<DeviceInfo>, WebcamError> {
    #[cfg(target_os = "windows")]
    {
        windows_impl::list_devices()
    }
    #[cfg(not(target_os = "windows"))]
    {
        // TODO macOS : AVFoundation
        // TODO Linux  : V4L2
        Ok(vec![DeviceInfo {
            id:   "0".to_string(),
            name: "Webcam (stub)".to_string(),
        }])
    }
}

/// Boucle principale du worker webcam.
/// Possède la Camera, écrit dans `pip_buffer` jusqu'à stop_flag.
pub fn run_worker(
    device_index: u32,
    stop_flag: Arc<AtomicBool>,
    pip_buffer: Arc<Mutex<Option<Frame>>>,
) {
    #[cfg(target_os = "windows")]
    {
        windows_impl::run_worker(device_index, stop_flag, pip_buffer);
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (device_index, stop_flag, pip_buffer);
        log::warn!("Capture webcam non implémentée sur cette plateforme");
    }
}

// ============================================================
// Helpers communs
// ============================================================

/// Convertit un buffer RGB packé (3 bytes/pixel) vers BGRA (4 bytes/pixel)
/// en redimensionnant via nearest-neighbor.
///
/// Pourquoi nearest et pas bilinéaire ? Sur une vignette 400×225, la perte
/// visuelle est imperceptible pour une webcam (qualité source déjà limitée),
/// et nearest tient à ~500 MB/s sur un CPU récent — largement assez pour
/// 30 fps de webcam même en 4K. Pour de la production cinéma il faudrait
/// bilinéaire/lanczos, hors scope ici.
pub fn rgb_to_bgra_nearest_resize(
    src_rgb: &[u8],
    src_w: u32,
    src_h: u32,
    dst_w: u32,
    dst_h: u32,
) -> Vec<u8> {
    let mut out = vec![0u8; (dst_w * dst_h * 4) as usize];
    if src_w == 0 || src_h == 0 || dst_w == 0 || dst_h == 0 {
        return out;
    }

    let x_ratio = src_w as f64 / dst_w as f64;
    let y_ratio = src_h as f64 / dst_h as f64;

    for dy in 0..dst_h {
        let sy = ((dy as f64 * y_ratio) as u32).min(src_h - 1);
        for dx in 0..dst_w {
            let sx = ((dx as f64 * x_ratio) as u32).min(src_w - 1);

            let src_idx = ((sy * src_w + sx) * 3) as usize;
            let dst_idx = ((dy * dst_w + dx) * 4) as usize;

            // src est RGB packé, dst est BGRA
            out[dst_idx]     = src_rgb[src_idx + 2]; // B
            out[dst_idx + 1] = src_rgb[src_idx + 1]; // G
            out[dst_idx + 2] = src_rgb[src_idx];     // R
            out[dst_idx + 3] = 255;                  // A opaque
        }
    }
    out
}

// ============================================================
// Implémentation Windows (nokhwa + MSMF)
// ============================================================
#[cfg(target_os = "windows")]
mod windows_impl {
    use super::*;

    use nokhwa::{
        pixel_format::RgbFormat,
        query,
        utils::{ApiBackend, CameraIndex, RequestedFormat, RequestedFormatType},
        Camera,
    };

    pub fn list_devices() -> Result<Vec<DeviceInfo>, WebcamError> {
        let cameras = query(ApiBackend::Auto)
            .map_err(|e| WebcamError::Enumeration(e.to_string()))?;

        let out: Vec<DeviceInfo> = cameras
            .into_iter()
            .map(|c| DeviceInfo {
                id:   format!("{}", c.index()),
                name: c.human_name(),
            })
            .collect();

        log::debug!("webcam::list_devices → {} caméra(s)", out.len());
        Ok(out)
    }

    pub fn run_worker(
        device_index: u32,
        stop_flag: Arc<AtomicBool>,
        pip_buffer: Arc<Mutex<Option<Frame>>>,
    ) {
        log::info!("Worker webcam · démarrage (index={device_index})");

        // 1. Ouvrir la caméra — format = framerate max disponible en RGB
        let format = RequestedFormat::new::<RgbFormat>(
            RequestedFormatType::AbsoluteHighestFrameRate,
        );

        let mut camera = match Camera::new(CameraIndex::Index(device_index), format) {
            Ok(c) => c,
            Err(e) => {
                log::warn!("Webcam désactivée : ouverture impossible ({e})");
                return;
            }
        };

        if let Err(e) = camera.open_stream() {
            log::warn!("Webcam désactivée : open_stream a échoué ({e})");
            return;
        }

        let resolution = camera.resolution();
        log::info!(
            "Webcam · ouverte ({}×{}) → PiP {}×{}",
            resolution.width(), resolution.height(),
            PIP_WIDTH, PIP_HEIGHT,
        );

        // 2. Boucle de capture
        let started_at = Instant::now();
        let mut consecutive_errors = 0u32;

        while !stop_flag.load(Ordering::Relaxed) {
            let buffer = match camera.frame() {
                Ok(b) => { consecutive_errors = 0; b }
                Err(e) => {
                    consecutive_errors += 1;
                    log::trace!("webcam.frame() : {e}");
                    if consecutive_errors > 30 {
                        log::error!("Webcam · {consecutive_errors} erreurs consécutives, abandon");
                        break;
                    }
                    std::thread::sleep(Duration::from_millis(33));
                    continue;
                }
            };

            // 3. Decode → image::ImageBuffer<Rgb<u8>, Vec<u8>>
            let rgb_image = match buffer.decode_image::<RgbFormat>() {
                Ok(i) => i,
                Err(e) => {
                    log::trace!("webcam decode : {e}");
                    continue;
                }
            };

            let src_w = rgb_image.width();
            let src_h = rgb_image.height();
            let rgb_data: &[u8] = rgb_image.as_raw();

            // 4. RGB → BGRA + resize vers dimensions PiP
            let bgra = rgb_to_bgra_nearest_resize(
                rgb_data, src_w, src_h, PIP_WIDTH, PIP_HEIGHT,
            );

            let pip_frame = Frame {
                width:  PIP_WIDTH,
                height: PIP_HEIGHT,
                stride: PIP_WIDTH * 4,
                data: bgra,
                timestamp: started_at.elapsed(),
            };

            // 5. Publier dans le buffer partagé. Le lock est tenu < 1 µs
            //    (juste un move). Pas de contention notable avec le worker vidéo.
            if let Ok(mut g) = pip_buffer.lock() {
                *g = Some(pip_frame);
            }
        }

        // 6. Fermeture propre
        if let Err(e) = camera.stop_stream() {
            log::warn!("camera.stop_stream() : {e}");
        }
        // Effacer le buffer pour que le worker vidéo arrête de blitter
        // un cadre figé si jamais il y a une frame résiduelle en transit.
        if let Ok(mut g) = pip_buffer.lock() {
            *g = None;
        }

        log::info!("Worker webcam · terminé");
    }
}
