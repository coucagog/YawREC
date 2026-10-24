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
// La vignette est en 16:9 et proportionnelle à la largeur de la vidéo
// produite (≈ 20 %), pour rester lisible en 4K comme sur une petite région.

/// Taille et marge de l'incrustation pour une vidéo de largeur `out_w`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PipLayout {
    pub width: u32,
    pub height: u32,
    pub margin: u32,
}

impl PipLayout {
    pub fn for_output_width(out_w: u32) -> Self {
        let width = ((out_w / 5).clamp(96, 960)) & !1;
        let height = (width * 9 / 16) & !1;
        let margin = (out_w / 80).clamp(8, 48);
        Self { width, height, margin }
    }
}

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
    layout: PipLayout,
    stop_flag: Arc<AtomicBool>,
    pip_buffer: Arc<Mutex<Option<Frame>>>,
) {
    #[cfg(target_os = "windows")]
    {
        windows_impl::run_worker(device_index, layout, stop_flag, pip_buffer);
    }
    #[cfg(not(target_os = "windows"))]
    {
        let _ = (device_index, layout, stop_flag, pip_buffer);
        log::warn!("Capture webcam non implémentée sur cette plateforme");
    }
}

// ============================================================
// Helpers communs
// ============================================================

/// Convertit un buffer RGB packé (3 bytes/pixel) vers BGRA (4 bytes/pixel)
/// en redimensionnant via nearest-neighbor, en mode "cover" : la source est
/// recadrée au centre pour avoir le ratio de la destination, ce qui évite
/// d'écraser une webcam 4:3 dans une vignette 16:9.
///
/// Nearest suffit pour une vignette (qualité source déjà limitée) et tient
/// largement 30 fps même pour une webcam 4K.
pub fn rgb_to_bgra_nearest_resize(
    src_rgb: &[u8],
    src_w: u32,
    src_h: u32,
    dst_w: u32,
    dst_h: u32,
) -> Vec<u8> {
    let mut out = vec![0u8; (dst_w * dst_h * 4) as usize];
    if src_w == 0 || src_h == 0 || dst_w == 0 || dst_h == 0
        || src_rgb.len() < (src_w * src_h * 3) as usize
    {
        return out;
    }

    // Zone source utilisée : le plus grand rectangle centré au ratio dst.
    let (crop_w, crop_h) = if (src_w as u64) * (dst_h as u64) > (src_h as u64) * (dst_w as u64) {
        (((src_h as u64 * dst_w as u64) / dst_h as u64) as u32, src_h)
    } else {
        (src_w, ((src_w as u64 * dst_h as u64) / dst_w as u64) as u32)
    };
    let crop_w = crop_w.max(1);
    let crop_h = crop_h.max(1);
    let x0 = (src_w - crop_w) / 2;
    let y0 = (src_h - crop_h) / 2;

    let x_ratio = crop_w as f64 / dst_w as f64;
    let y_ratio = crop_h as f64 / dst_h as f64;

    for dy in 0..dst_h {
        let sy = (y0 + (dy as f64 * y_ratio) as u32).min(src_h - 1);
        for dx in 0..dst_w {
            let sx = (x0 + (dx as f64 * x_ratio) as u32).min(src_w - 1);

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
        layout: PipLayout,
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
            layout.width, layout.height,
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
                rgb_data, src_w, src_h, layout.width, layout.height,
            );

            let pip_frame = Frame {
                width:  layout.width,
                height: layout.height,
                stride: layout.width * 4,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pip_layout_proportionnel() {
        let l = PipLayout::for_output_width(1920);
        assert_eq!((l.width, l.height, l.margin), (384, 216, 24));
        let l4k = PipLayout::for_output_width(3840);
        assert_eq!(l4k.width, 768);
        let petit = PipLayout::for_output_width(300);
        assert_eq!(petit.width, 96);
        assert_eq!(petit.width % 2, 0);
        assert_eq!(petit.height % 2, 0);
    }

    #[test]
    fn resize_cover_recadre_le_centre() {
        // Source 4×2 : colonnes 0 et 3 rouges, 1 et 2 vertes. Destination 1:1
        // → seules les colonnes centrales (vertes) doivent être gardées.
        let mut src = Vec::new();
        for _y in 0..2 {
            for x in 0..4 {
                if x == 0 || x == 3 { src.extend_from_slice(&[255, 0, 0]); }
                else { src.extend_from_slice(&[0, 255, 0]); }
            }
        }
        let out = rgb_to_bgra_nearest_resize(&src, 4, 2, 2, 2);
        for px in out.chunks_exact(4) {
            assert_eq!(px, &[0, 255, 0, 255]);
        }
    }
}
