// ============================================================
// YAWREC · state.rs
// État partagé entre les commands Tauri et les workers.
//
// Nouveautés D4 + D5 :
//   - `webcam_enabled` : flip qui décide si on spawne le worker webcam
//     au prochain start_recording.
//   - `pip_buffer` : Arc<Mutex<Option<Frame>>> partagé entre le worker
//     webcam (producteur, écrit à ~30 Hz) et le worker vidéo (consommateur,
//     lit à ~60 Hz pour composer en PiP). Toujours None tant que le worker
//     webcam n'a pas reçu sa première frame.
//   - `webcam_worker` : JoinHandle du thread webcam pour join propre.
// ============================================================

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

use serde::{Deserialize, Serialize};

use crate::capture::Frame;
use crate::encoder::Encoder;

/// Position de l'incrustation webcam dans la frame écran.
#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PipPosition {
    TopLeft,
    TopRight,
    BottomLeft,
    #[default]
    BottomRight,
}

impl PipPosition {
    pub fn from_u8(v: u8) -> Self {
        match v {
            0 => PipPosition::TopLeft,
            1 => PipPosition::TopRight,
            2 => PipPosition::BottomLeft,
            _ => PipPosition::BottomRight,
        }
    }
    pub fn to_u8(self) -> u8 {
        match self {
            PipPosition::TopLeft     => 0,
            PipPosition::TopRight    => 1,
            PipPosition::BottomLeft  => 2,
            PipPosition::BottomRight => 3,
        }
    }
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum CaptureMode {
    #[default]
    Fullscreen,
    Window,
    Region,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum RecordingPhase {
    #[default]
    Idle,
    Recording,
    Paused,
}

pub struct RecorderState {
    pub phase: RecordingPhase,
    pub started_at: Option<Instant>,
    pub paused_offset: Duration,

    pub mode: CaptureMode,
    pub output_dir: Option<PathBuf>,
    pub screen_id: Option<u32>,
    /// HWND de la fenêtre sélectionnée pour le mode Window (i64 = isize cross-plateforme).
    pub selected_hwnd: Option<i64>,
    /// Zone sélectionnée pour le mode Region (x, y, w, h en pixels physiques).
    pub selected_region: Option<(i32, i32, u32, u32)>,
    /// Qualité vidéo : cadence cible et bitrate.
    pub video_fps: u32,
    pub video_bitrate_kbps: u32,

    /// Audio
    pub mic_enabled: bool,
    pub loopback_enabled: bool,
    pub mic_device_name: Option<String>, // None = device par défaut

    /// D4 — webcam ON/OFF. Décidé avant chaque start_recording.
    pub webcam_enabled: bool,
    pub webcam_device_id: Option<u32>,

    pub current_output_path: Option<PathBuf>,

    // ----- partagés avec les workers -----
    pub byte_count:        Arc<AtomicU64>,
    pub frame_count:       Arc<AtomicU64>,
    pub stop_flag:         Arc<AtomicBool>,
    pub audio_paused:      Arc<AtomicBool>,
    /// Accumulated milliseconds spent in pause (reset at each start_recording).
    pub paused_total_ms:   Arc<AtomicU64>,
    /// Wall-clock instant when the current pause began (None when not paused).
    pub pause_started_at:  Option<Instant>,

    /// Encoder partagé : Some(Encoder) après lazy-init par le worker vidéo.
    pub encoder_arc: Option<Arc<Mutex<Option<Encoder>>>>,

    /// D5 — frame webcam déjà mise à la taille PiP (BGRA8). Le worker vidéo
    /// la blitte sur chaque frame écran avant push à l'encoder.
    pub pip_buffer: Option<Arc<Mutex<Option<Frame>>>>,
    /// Position de l'incrustation (partagée avec le worker vidéo via AtomicU8).
    pub pip_position: Arc<AtomicU8>,
    /// Gain linéaire appliqué aux samples micro avant mixage (f32 stocké en bits).
    /// 1.0 = unité, 0.0 = muet, 2.0 = +6 dB. Modifiable en live.
    pub mic_gain: Arc<AtomicU32>,
    /// Niveau RMS du micro (f32 bits, 0.0–1.0). Mis à jour par l'AudioMonitor
    /// (repos) ou le worker audio (enregistrement). Lu par la boucle tick → VU.
    pub mic_level: Arc<AtomicU32>,
    /// Flag d'arrêt du thread AudioMonitor courant.
    pub mic_monitor_stop: Arc<AtomicBool>,
    /// Handle du thread AudioMonitor. None quand l'enregistrement est actif.
    pub mic_monitor_handle: Option<JoinHandle<()>>,

    pub video_worker:  Option<JoinHandle<()>>,
    pub audio_worker:  Option<JoinHandle<()>>,
    pub webcam_worker: Option<JoinHandle<()>>,
}

impl Default for RecorderState {
    fn default() -> Self {
        Self {
            phase: RecordingPhase::default(),
            started_at: None,
            paused_offset: Duration::ZERO,
            mode: CaptureMode::default(),
            output_dir: None,
            screen_id: None,
            selected_hwnd: None,
            selected_region: None,
            video_fps: 30,
            video_bitrate_kbps: 8000,
            mic_enabled: true,
            loopback_enabled: false,
            mic_device_name: None,
            webcam_enabled: false,
            webcam_device_id: None,
            current_output_path: None,
            byte_count:       Arc::new(AtomicU64::new(0)),
            frame_count:      Arc::new(AtomicU64::new(0)),
            stop_flag:        Arc::new(AtomicBool::new(false)),
            audio_paused:     Arc::new(AtomicBool::new(false)),
            paused_total_ms:  Arc::new(AtomicU64::new(0)),
            pause_started_at: None,
            encoder_arc: None,
            pip_buffer: None,
            pip_position: Arc::new(AtomicU8::new(PipPosition::BottomRight.to_u8())),
            mic_gain:  Arc::new(AtomicU32::new(1.0f32.to_bits())),
            mic_level: Arc::new(AtomicU32::new(0f32.to_bits())),
            mic_monitor_stop:   Arc::new(AtomicBool::new(false)),
            mic_monitor_handle: None,
            video_worker:  None,
            audio_worker:  None,
            webcam_worker: None,
        }
    }
}

impl RecorderState {
    pub fn elapsed(&self) -> Duration {
        match (self.phase, self.started_at) {
            (RecordingPhase::Recording, Some(t)) => self.paused_offset + t.elapsed(),
            (RecordingPhase::Paused, _) => self.paused_offset,
            _ => Duration::ZERO,
        }
    }

    pub fn format_elapsed(&self) -> String {
        let total = self.elapsed().as_secs();
        format!("{:02}:{:02}:{:02}", total / 3600, (total / 60) % 60, total % 60)
    }

    pub fn compute_output_path(&self) -> PathBuf {
        let stamp = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_secs())
            .unwrap_or(0);
        let filename = format!("YawREC-{stamp}.mp4");
        match &self.output_dir {
            Some(d) => d.join(filename),
            None => {
                // Default: %USERPROFILE%\Videos\YawREC
                let base = std::env::var("USERPROFILE")
                    .map(PathBuf::from)
                    .unwrap_or_else(|_| std::env::temp_dir());
                let dir = base.join("Videos").join("YawREC");
                let _ = std::fs::create_dir_all(&dir);
                dir.join(filename)
            }
        }
    }

    pub fn output_dir_display(&self) -> String {
        match &self.output_dir {
            Some(d) => d.to_string_lossy().into_owned(),
            None => {
                let base = std::env::var("USERPROFILE")
                    .map(PathBuf::from)
                    .unwrap_or_else(|_| std::env::temp_dir());
                base.join("Videos").join("YawREC").to_string_lossy().into_owned()
            }
        }
    }
}
