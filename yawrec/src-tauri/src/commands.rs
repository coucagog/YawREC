// ============================================================
// YAWREC · commands.rs
// Commandes Tauri (IPC) + workers + helpers partagés.
//
// Architecture D4 + D5 :
//   Le worker vidéo reçoit en plus un Option<Arc<Mutex<Option<Frame>>>>
//   (pip_buffer). Si présent et non-vide, il blitte la frame webcam sur
//   chaque frame écran en bas-droite avant de la pousser à l'encoder.
//   Le worker webcam est spawné en parallèle si webcam_enabled est true
//   au moment du start_recording.
// ============================================================

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_notification::NotificationExt;

use crate::capture::audio::AudioCapturer;
use crate::capture::webcam::PIP_MARGIN;
use crate::capture::window::WindowInfo;
use crate::capture::{audio, screen, webcam, window, Frame};
use crate::encoder::{Encoder, EncoderConfig, VideoEncoder};
use crate::error::{YawrecError, YawrecResult};
use crate::state::{CaptureMode, PipPosition, RecorderState, RecordingPhase};

// ============================================================
// Payloads IPC
// ============================================================

#[derive(Debug, Serialize, Clone)]
pub struct DeviceInfo {
    pub id: String,
    pub name: String,
}

#[derive(Debug, Serialize, Clone)]
pub struct ScreenInfo {
    pub id: u32,
    pub name: String,
    pub width: u32,
    pub height: u32,
    pub primary: bool,
}

#[derive(Debug, Serialize, Clone)]
pub struct StatusPayload {
    pub phase: RecordingPhase,
    pub elapsed: String,
    pub size_bytes: u64,
    pub size_human: String,
    pub frame_count: u64,
}

impl StatusPayload {
    pub fn from_state(s: &RecorderState) -> Self {
        let size = s.byte_count.load(Ordering::Relaxed);
        let frames = s.frame_count.load(Ordering::Relaxed);
        Self {
            phase: s.phase,
            elapsed: s.format_elapsed(),
            size_bytes: size,
            size_human: human_size(size),
            frame_count: frames,
        }
    }
}

fn human_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let b = bytes as f64;
    if b >= GB      { format!("{:.2} GB", b / GB) }
    else if b >= MB { format!("{:.2} MB", b / MB) }
    else if b >= KB { format!("{:.0} KB", b / KB) }
    else            { format!("{} B", bytes) }
}

// ============================================================
// D5 — Compositing PiP webcam → écran
// ============================================================

/// Calcule le coin haut-gauche de la vignette PiP selon la position choisie.
fn pip_offset(screen_w: u32, screen_h: u32, pip_w: u32, pip_h: u32, pos: PipPosition) -> (usize, usize) {
    let m = PIP_MARGIN as usize;
    let x = match pos {
        PipPosition::TopLeft | PipPosition::BottomLeft => m,
        PipPosition::TopRight | PipPosition::BottomRight =>
            (screen_w as usize).saturating_sub(pip_w as usize).saturating_sub(m),
    };
    let y = match pos {
        PipPosition::TopLeft | PipPosition::TopRight => m,
        PipPosition::BottomLeft | PipPosition::BottomRight =>
            (screen_h as usize).saturating_sub(pip_h as usize).saturating_sub(m),
    };
    (x, y)
}

/// Blitte la frame PiP dans le coin choisi de la frame écran.
///
/// Hypothèses :
///   - screen et pip sont en BGRA8 packé, sans padding ligne (stride = w*4)
///   - le PiP est plus petit que l'écran + 2 × MARGIN ; sinon on no-op
///   - opacité = 1.0 (pas d'alpha blending) — on overwrite simplement
///
/// Coût : ~360 KB memcpy à 60 fps = 22 MB/s, négligeable.
fn composite_pip(screen: &mut [u8], screen_w: u32, screen_h: u32, pip: &Frame, pos: PipPosition) {
    // Vérifications de borne : si l'écran est trop petit pour héberger
    // le PiP avec sa marge, on skip silencieusement.
    let total_pip_w = pip.width + 2 * PIP_MARGIN;
    let total_pip_h = pip.height + 2 * PIP_MARGIN;
    if screen_w < total_pip_w || screen_h < total_pip_h {
        return;
    }

    let (x_off, y_off) = pip_offset(screen_w, screen_h, pip.width, pip.height, pos);
    let row_bytes = (pip.width * 4) as usize;
    let screen_stride = (screen_w * 4) as usize;

    for y in 0..pip.height as usize {
        let dst_start = (y_off + y) * screen_stride + x_off * 4;
        let src_start = y * row_bytes;

        let dst_end = dst_start + row_bytes;
        let src_end = src_start + row_bytes;
        if dst_end > screen.len() || src_end > pip.data.len() {
            break;
        }
        screen[dst_start..dst_end].copy_from_slice(&pip.data[src_start..src_end]);
    }

    // Pas de bordure — supprimée à la demande (D5-b).
    // draw_pip_border() conservée ci-dessous pour usage futur (fond coloré, ombre, etc.).
}

fn draw_pip_border(
    screen: &mut [u8],
    screen_w: u32,
    screen_h: u32,
    pip_w: u32,
    pip_h: u32,
    pos: PipPosition,
) {
    const THICKNESS: u32 = 2;
    let (x_off_usize, y_off_usize) = pip_offset(screen_w, screen_h, pip_w, pip_h, pos);
    let x_off = x_off_usize as u32;
    let y_off = y_off_usize as u32;
    let stride = (screen_w * 4) as usize;

    // Top et bottom : THICKNESS lignes pleines de (pip_w + 2*THICKNESS) px
    // Left et right : THICKNESS colonnes
    let x0 = x_off.saturating_sub(THICKNESS);
    let y0 = y_off.saturating_sub(THICKNESS);
    let x1 = (x_off + pip_w + THICKNESS - 1).min(screen_w - 1);
    let y1 = (y_off + pip_h + THICKNESS - 1).min(screen_h - 1);

    let mut set_px = |x: u32, y: u32| {
        let idx = (y as usize) * stride + (x as usize) * 4;
        if idx + 3 < screen.len() {
            screen[idx]     = 255; // B
            screen[idx + 1] = 255; // G
            screen[idx + 2] = 255; // R
            screen[idx + 3] = 255;
        }
    };

    // Top + bottom bands
    for t in 0..THICKNESS {
        let yt = y0 + t;
        let yb = y1.saturating_sub(t);
        for x in x0..=x1 {
            set_px(x, yt);
            set_px(x, yb);
        }
    }
    // Left + right bands
    for t in 0..THICKNESS {
        let xl = x0 + t;
        let xr = x1.saturating_sub(t);
        for y in y0..=y1 {
            set_px(xl, y);
            set_px(xr, y);
        }
    }
}

// ============================================================
// D6 — Crop de frame pour le mode Window
// ============================================================

/// Extrait un sous-rectangle (wx, wy, ww, wh) d'une frame plein écran BGRA8.
/// Les pixels hors limites de la frame source restent noirs (zero-init).
fn crop_frame_to_window(frame: &Frame, wx: i32, wy: i32, ww: u32, wh: u32) -> Frame {
    let screen_w = frame.width as i32;
    let screen_h = frame.height as i32;
    let screen_stride = (frame.width * 4) as usize;
    let dst_row_bytes = (ww * 4) as usize;
    let mut data = vec![0u8; dst_row_bytes * wh as usize];

    for row in 0..wh as i32 {
        let screen_y = wy + row;
        if screen_y < 0 || screen_y >= screen_h {
            continue;
        }

        // Plage de colonnes visible dans la frame source
        let col_start = (-wx).max(0) as u32;
        let col_end = (screen_w - wx).min(ww as i32).max(0) as u32;
        if col_start >= col_end {
            continue;
        }

        let src_x = (wx + col_start as i32) as usize;
        let src_off = screen_y as usize * screen_stride + src_x * 4;
        let dst_off = row as usize * dst_row_bytes + col_start as usize * 4;
        let n = (col_end - col_start) as usize * 4;

        data[dst_off..dst_off + n].copy_from_slice(&frame.data[src_off..src_off + n]);
    }

    Frame {
        width: ww,
        height: wh,
        stride: ww * 4,
        data,
        timestamp: frame.timestamp,
    }
}

// ============================================================
// Worker vidéo
// ============================================================

struct VideoWorkerCtx {
    screen_id: u32,
    output_path: PathBuf,
    stop_flag: Arc<AtomicBool>,
    pause_flag: Arc<AtomicBool>,
    paused_total_ms: Arc<AtomicU64>,
    byte_count: Arc<AtomicU64>,
    frame_count: Arc<AtomicU64>,
    encoder_arc: Arc<Mutex<Option<Encoder>>>,
    pip_buffer: Option<Arc<Mutex<Option<Frame>>>>,
    pip_position: Arc<AtomicU8>,
    /// D6 — mode Window : découpe la frame DXGI aux dimensions de la fenêtre.
    capture_mode: CaptureMode,
    selected_hwnd: Option<i64>,
    /// Dimensions fixes de la zone de crop (déterminées au démarrage de l'enregistrement).
    crop_w: u32,
    crop_h: u32,
    app: AppHandle,
}

fn run_video_worker(ctx: VideoWorkerCtx) {
    use crate::capture::Capturer;

    log::info!(
        "Worker vidéo · démarrage (screen={}, mode={:?}, pip={}, crop={}×{})",
        ctx.screen_id,
        ctx.capture_mode,
        ctx.pip_buffer.is_some(),
        ctx.crop_w,
        ctx.crop_h,
    );

    let mut capturer = match screen::make_capturer(ctx.screen_id) {
        Ok(c) => c,
        Err(e) => {
            log::error!("make_capturer : {e}");
            let _ = ctx.app.emit("recorder://error", format!("Capture écran : {e}"));
            return;
        }
    };
    if let Err(e) = capturer.start() {
        log::error!("capturer.start() : {e}");
        let _ = ctx.app.emit("recorder://error", format!("Démarrage capture : {e}"));
        return;
    }

    while !ctx.stop_flag.load(Ordering::Relaxed) {
        match capturer.next_frame() {
            Ok(Some(mut frame)) => {
                // Drop frames while paused
                if ctx.pause_flag.load(Ordering::Relaxed) {
                    continue;
                }

                // D6 — Window mode : recadrer la frame sur la fenêtre sélectionnée.
                // On récupère la position courante à chaque frame pour suivre les déplacements.
                if ctx.capture_mode == CaptureMode::Window && ctx.crop_w > 0 {
                    if let Some(hwnd) = ctx.selected_hwnd {
                        if let Some((wx, wy, _, _)) = window::get_window_rect(hwnd) {
                            frame = crop_frame_to_window(&frame, wx, wy, ctx.crop_w, ctx.crop_h);
                        }
                    }
                }

                // D5 — composite PiP avant encodage
                if let Some(pip_arc) = &ctx.pip_buffer {
                    if let Ok(guard) = pip_arc.lock() {
                        if let Some(pip) = guard.as_ref() {
                            let pos = PipPosition::from_u8(ctx.pip_position.load(Ordering::Relaxed));
                            composite_pip(&mut frame.data, frame.width, frame.height, pip, pos);
                        }
                    }
                }

                let mut guard = match ctx.encoder_arc.lock() {
                    Ok(g) => g,
                    Err(p) => p.into_inner(),
                };

                if guard.is_none() {
                    let config = EncoderConfig {
                        output_path: ctx.output_path.clone(),
                        width: frame.width,
                        height: frame.height,
                        ..Default::default()
                    };
                    match Encoder::new(config) {
                        Ok(e) => *guard = Some(e),
                        Err(e) => {
                            log::error!("Encoder init : {e}");
                            let _ = ctx.app.emit("recorder://error", format!("Encodeur : {e}"));
                            break;
                        }
                    }
                }

                let enc = guard.as_mut().unwrap();
                // Subtract accumulated pause duration so PTS stays continuous
                let raw_ms = frame.timestamp.as_millis() as u64;
                let paused_ms = ctx.paused_total_ms.load(Ordering::Relaxed);
                let ts_ms = raw_ms.saturating_sub(paused_ms);
                if let Err(e) = enc.push_video_frame(&frame.data, ts_ms) {
                    log::error!("push_video_frame : {e}");
                    break;
                }

                ctx.frame_count.fetch_add(1, Ordering::Relaxed);
                ctx.byte_count.store(enc.bytes_written(), Ordering::Relaxed);
            }
            // DXGI AcquireNextFrame timeout (100 ms) ou duplication fermée.
            // On continue le polling ; la condition while vérifie stop_flag.
            Ok(None) => { continue; }
            Err(e) => {
                    log::error!("next_frame : {e}");
                    let _ = ctx.app.emit("recorder://error", format!("Capture frame : {e}"));
                    break;
                }
        }
    }

    if let Err(e) = capturer.stop() {
        log::warn!("capturer.stop() : {e}");
    }

    // Si le stop_flag n'était pas positionné, la boucle s'est arrêtée sur une erreur.
    // Le message d'erreur a déjà été émis ci-dessus ; pas besoin de doubler.
    log::info!("Worker vidéo · terminé (frames={})", ctx.frame_count.load(Ordering::Relaxed));
}

// ============================================================
// Worker audio
// ============================================================

fn run_audio_worker(
    stop_flag: Arc<AtomicBool>,
    paused: Arc<AtomicBool>,
    encoder_arc: Arc<Mutex<Option<Encoder>>>,
    mic_enabled: bool,
    loopback_enabled: bool,
    mic_device_name: Option<String>,
    mic_gain:  Arc<AtomicU32>,
    mic_level: Arc<AtomicU32>,
) {
    if !mic_enabled && !loopback_enabled {
        log::info!("Worker audio · désactivé (mic et loopback OFF)");
        return;
    }

    log::info!("Worker audio · démarrage (mic={mic_enabled}, loopback={loopback_enabled})");

    let capturer = match AudioCapturer::open(mic_enabled, loopback_enabled, mic_device_name.as_deref(), mic_gain, mic_level) {
        Ok(c) => c,
        Err(e) => { log::warn!("Audio désactivé : {e}"); return; }
    };

    log::info!(
        "Audio sources : micro={}, loopback={}",
        capturer.has_mic(), capturer.has_loopback(),
    );

    let mut next_poll = Instant::now();
    let poll_interval = Duration::from_millis(10);

    while !stop_flag.load(Ordering::Relaxed) {
        let now = Instant::now();
        if now < next_poll {
            std::thread::sleep(next_poll - now);
        }
        next_poll += poll_interval;

        if paused.load(Ordering::Relaxed) {
            let _ = capturer.pull_chunk();
            continue;
        }

        let chunk = match capturer.pull_chunk() {
            Some(c) => c,
            None => continue,
        };

        let mut guard = match encoder_arc.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        };
        if let Some(enc) = guard.as_mut() {
            if let Err(e) = enc.push_audio_samples(&chunk, 0) {
                log::error!("push_audio_samples : {e}");
                break;
            }
        }
    }

    log::info!("Worker audio · terminé");
}

// ============================================================
// Helpers partagés (IPC + raccourci F1 + menu tray F4)
// ============================================================

pub async fn do_start_recording(app: &AppHandle) -> YawrecResult<()> {
    let state_mutex = app.state::<Mutex<RecorderState>>();

    // Lock court : prépare l'état, clone les Arcs pour les workers
    let (stop_flag, audio_paused, paused_total_ms, byte_count, frame_count,
         screen_id, output_path, encoder_arc,
         pip_buffer, webcam_idx, pip_position,
         mic_enabled, loopback_enabled, mic_device_name, mic_gain, mic_level,
         capture_mode, selected_hwnd, crop_w, crop_h,
         mic_monitor_handle_opt) = {
        let mut s = state_mutex.lock().unwrap();
        if s.phase != RecordingPhase::Idle {
            return Err(YawrecError::InvalidState(
                "Un enregistrement est déjà en cours".into(),
            ));
        }
        s.phase = RecordingPhase::Recording;
        s.started_at = Some(Instant::now());
        s.paused_offset = Duration::ZERO;
        s.pause_started_at = None;
        s.byte_count.store(0, Ordering::Relaxed);
        s.frame_count.store(0, Ordering::Relaxed);
        s.stop_flag.store(false, Ordering::Relaxed);
        s.audio_paused.store(false, Ordering::Relaxed);
        s.paused_total_ms.store(0, Ordering::Relaxed);

        let out = s.compute_output_path();
        s.current_output_path = Some(out.clone());

        let enc_arc: Arc<Mutex<Option<Encoder>>> = Arc::new(Mutex::new(None));
        s.encoder_arc = Some(Arc::clone(&enc_arc));

        // D5 — pip buffer si webcam activée
        let (pip_buf, webcam_idx) = if s.webcam_enabled {
            let buf: Arc<Mutex<Option<Frame>>> = Arc::new(Mutex::new(None));
            s.pip_buffer = Some(Arc::clone(&buf));
            (Some(buf), s.webcam_device_id.unwrap_or(0))
        } else {
            s.pip_buffer = None;
            (None, 0)
        };

        // D6 — Window mode : dimensions de crop fixées au démarrage.
        let (crop_w, crop_h) = if s.mode == CaptureMode::Window {
            match s.selected_hwnd.and_then(|h| window::get_window_rect(h)) {
                Some((_, _, w, h)) => (w & !1, h & !1), // arrondir à pair pour H.264
                None => (0, 0),
            }
        } else {
            (0, 0)
        };

        (
            Arc::clone(&s.stop_flag),
            Arc::clone(&s.audio_paused),
            Arc::clone(&s.paused_total_ms),
            Arc::clone(&s.byte_count),
            Arc::clone(&s.frame_count),
            s.screen_id.unwrap_or(0),
            out,
            enc_arc,
            pip_buf,
            webcam_idx,
            Arc::clone(&s.pip_position),
            s.mic_enabled,
            s.loopback_enabled,
            s.mic_device_name.clone(),
            Arc::clone(&s.mic_gain),
            Arc::clone(&s.mic_level),
            s.mode,
            s.selected_hwnd,
            crop_w,
            crop_h,
            // Arrêt du monitor VU : le worker audio prend le relais
            { s.mic_monitor_stop.store(true, Ordering::Relaxed); s.mic_monitor_handle.take() },
        )
    };

    // Laisser le thread monitor s'arrêter naturellement (il vérifie le flag toutes les 100 ms).
    drop(mic_monitor_handle_opt);

    // ---- Worker vidéo ----
    #[cfg(target_os = "windows")]
    let video_handle = {
        let ctx = VideoWorkerCtx {
            screen_id,
            output_path,
            stop_flag: Arc::clone(&stop_flag),
            pause_flag: Arc::clone(&audio_paused),
            paused_total_ms: Arc::clone(&paused_total_ms),
            byte_count,
            frame_count,
            encoder_arc: Arc::clone(&encoder_arc),
            pip_buffer: pip_buffer.as_ref().map(Arc::clone),
            pip_position: Arc::clone(&pip_position),
            capture_mode,
            selected_hwnd,
            crop_w,
            crop_h,
            app: app.clone(),
        };
        std::thread::Builder::new()
            .name("yawrec-video-worker".into())
            .spawn(move || run_video_worker(ctx))
            .map_err(|e| YawrecError::Capture(format!("spawn vidéo : {e}")))?
    };

    #[cfg(not(target_os = "windows"))]
    let video_handle = {
        let _ = (screen_id, output_path, byte_count, frame_count, &encoder_arc, &stop_flag, &pip_buffer, capture_mode, selected_hwnd, crop_w, crop_h);
        log::warn!("Capture écran non implémentée sur cette plateforme — worker vidéo no-op");
        std::thread::Builder::new()
            .name("yawrec-video-worker-noop".into())
            .spawn(|| {})
            .map_err(|e| YawrecError::Capture(format!("spawn vidéo noop : {e}")))?
    };

    // ---- Worker audio ----
    let audio_handle = {
        let enc_arc = Arc::clone(&encoder_arc);
        let stop = Arc::clone(&stop_flag);
        let pause = Arc::clone(&audio_paused);
        std::thread::Builder::new()
            .name("yawrec-audio-worker".into())
            .spawn(move || run_audio_worker(stop, pause, enc_arc, mic_enabled, loopback_enabled, mic_device_name, mic_gain, mic_level))
            .map_err(|e| YawrecError::Capture(format!("spawn audio : {e}")))?
    };

    // ---- Worker webcam (D4) ----
    let has_webcam = pip_buffer.is_some();
    let webcam_handle = if let Some(pip_buf) = pip_buffer {
        let stop = Arc::clone(&stop_flag);
        Some(
            std::thread::Builder::new()
                .name("yawrec-webcam-worker".into())
                .spawn(move || webcam::run_worker(webcam_idx, stop, pip_buf))
                .map_err(|e| YawrecError::Capture(format!("spawn webcam : {e}")))?,
        )
    } else {
        None
    };

    {
        let mut s = state_mutex.lock().unwrap();
        s.video_worker  = Some(video_handle);
        s.audio_worker  = Some(audio_handle);
        s.webcam_worker = webcam_handle;
    }

    // F3 — notification
    let webcam_label = if has_webcam { " · webcam ON" } else { "" };
    notify(app, "YawREC", &format!("● Enregistrement démarré{webcam_label}"));

    log::info!("▶ start_recording (screen={screen_id}, webcam={has_webcam})");
    Ok(())
}

pub async fn do_stop_recording(app: &AppHandle) -> YawrecResult<String> {
    let state_mutex = app.state::<Mutex<RecorderState>>();

    let (video_handle, audio_handle, webcam_handle, encoder_arc, fallback_path) = {
        let mut s = state_mutex.lock().unwrap();
        if s.phase == RecordingPhase::Idle {
            return Err(YawrecError::InvalidState(
                "Aucun enregistrement en cours".into(),
            ));
        }
        s.phase = RecordingPhase::Idle;
        s.started_at = None;
        s.paused_offset = Duration::ZERO;
        s.stop_flag.store(true, Ordering::Relaxed);

        let path = s.current_output_path.take().unwrap_or_default();
        let result = (
            s.video_worker.take(),
            s.audio_worker.take(),
            s.webcam_worker.take(),
            s.encoder_arc.take(),
            path.to_string_lossy().to_string(),
        );
        s.pip_buffer = None;
        result
    };

    // Join hors lock
    if let Some(h) = video_handle {
        if let Err(e) = tokio::task::spawn_blocking(move || h.join()).await {
            log::warn!("join video worker : {e}");
        }
    }
    if let Some(h) = audio_handle {
        if let Err(e) = tokio::task::spawn_blocking(move || h.join()).await {
            log::warn!("join audio worker : {e}");
        }
    }
    if let Some(h) = webcam_handle {
        if let Err(e) = tokio::task::spawn_blocking(move || h.join()).await {
            log::warn!("join webcam worker : {e}");
        }
    }

    // Flush + close encoder
    let final_path = if let Some(enc_arc) = encoder_arc {
        let mut guard = enc_arc.lock().unwrap();
        match guard.as_mut() {
            Some(enc) => match enc.stop() {
                Ok(p) => p.to_string_lossy().to_string(),
                Err(e) => { log::error!("encoder.stop() : {e}"); fallback_path }
            },
            None => {
                log::warn!("Aucune frame capturée — pas de MP4 produit");
                let _ = app.emit(
                    "recorder://error",
                    "Aucune frame capturée. Vérifiez les permissions de capture d'écran.",
                );
                return Err(YawrecError::Capture(
                    "Aucune frame capturée — enregistrement vide".into(),
                ));
            }
        }
    } else {
        fallback_path
    };

    log::info!("■ stop_recording → {}", final_path);
    let _ = app.emit("recorder://stopped", &final_path);
    notify(
        app,
        "YawREC · enregistrement terminé",
        &format!("Fichier : {final_path}"),
    );

    // Redémarrer le monitor VU maintenant que le worker audio est arrêté.
    let (mic_enabled, mic_device, mic_level, mic_gain) = {
        let s = state_mutex.lock().unwrap();
        s.mic_level.store(0f32.to_bits(), Ordering::Relaxed);
        (s.mic_enabled, s.mic_device_name.clone(), Arc::clone(&s.mic_level), Arc::clone(&s.mic_gain))
    };
    if mic_enabled {
        let new_stop = Arc::new(AtomicBool::new(false));
        if let Ok(handle) = audio::start_monitor(
            mic_device.as_deref(), mic_level, mic_gain, Arc::clone(&new_stop),
        ) {
            let mut s = state_mutex.lock().unwrap();
            s.mic_monitor_stop = new_stop;
            s.mic_monitor_handle = Some(handle);
        }
    }

    Ok(final_path)
}

/// Toggle pause/reprise global (utilisé par le raccourci Ctrl+Shift+P).
pub async fn do_pause_recording(app: AppHandle) {
    let state_mutex = app.state::<Mutex<RecorderState>>();
    let phase = state_mutex.lock().unwrap().phase;
    match phase {
        RecordingPhase::Recording => {
            let state = app.state::<Mutex<RecorderState>>();
            if let Err(e) = (async {
                let mut s = state.lock().unwrap();
                if let Some(t) = s.started_at.take() {
                    s.paused_offset += t.elapsed();
                }
                s.pause_started_at = Some(Instant::now());
                s.phase = RecordingPhase::Paused;
                s.audio_paused.store(true, Ordering::Relaxed);
                log::info!("⏸ pause via raccourci");
                Ok::<(), YawrecError>(())
            }).await {
                log::error!("pause_recording : {e}");
            }
        }
        RecordingPhase::Paused => {
            let state = app.state::<Mutex<RecorderState>>();
            let mut s = state.lock().unwrap();
            if let Some(pause_start) = s.pause_started_at.take() {
                let paused_ms = pause_start.elapsed().as_millis() as u64;
                s.paused_total_ms.fetch_add(paused_ms, Ordering::Relaxed);
            }
            s.phase = RecordingPhase::Recording;
            s.started_at = Some(Instant::now());
            s.audio_paused.store(false, Ordering::Relaxed);
            log::info!("⏵ reprise via raccourci");
        }
        RecordingPhase::Idle => {}
    }
}

pub async fn do_toggle_recording(app: AppHandle) {
    let phase = {
        let state_mutex = app.state::<Mutex<RecorderState>>();
        let s = state_mutex.lock().unwrap();
        s.phase
    };
    match phase {
        RecordingPhase::Idle => {
            if let Err(e) = do_start_recording(&app).await {
                log::error!("toggle start : {e}");
                notify(&app, "YawREC", &format!("Erreur : {e}"));
            }
        }
        RecordingPhase::Recording | RecordingPhase::Paused => {
            if let Err(e) = do_stop_recording(&app).await {
                log::error!("toggle stop : {e}");
                notify(&app, "YawREC", &format!("Erreur : {e}"));
            }
        }
    }
}

fn notify(app: &AppHandle, title: &str, body: &str) {
    if let Err(e) = app.notification().builder().title(title).body(body).show() {
        log::debug!("notif ignorée : {e}");
    }
}

// ============================================================
// Commandes IPC : wrappers minces
// ============================================================

#[tauri::command]
pub async fn start_recording(
    app: AppHandle,
    _state: State<'_, Mutex<RecorderState>>,
) -> YawrecResult<()> {
    do_start_recording(&app).await
}

#[tauri::command]
pub async fn stop_recording(
    app: AppHandle,
    _state: State<'_, Mutex<RecorderState>>,
) -> YawrecResult<String> {
    do_stop_recording(&app).await
}

#[tauri::command]
pub async fn pause_recording(
    state: State<'_, Mutex<RecorderState>>,
) -> YawrecResult<()> {
    let mut s = state.lock().unwrap();
    if s.phase != RecordingPhase::Recording {
        return Err(YawrecError::InvalidState(
            "Pas d'enregistrement actif à mettre en pause".into(),
        ));
    }
    if let Some(t) = s.started_at.take() {
        s.paused_offset += t.elapsed();
    }
    s.phase = RecordingPhase::Paused;
    s.audio_paused.store(true, Ordering::Relaxed);
    log::info!("⏸ pause_recording");
    Ok(())
}

#[tauri::command]
pub async fn resume_recording(
    state: State<'_, Mutex<RecorderState>>,
) -> YawrecResult<()> {
    let mut s = state.lock().unwrap();
    if s.phase != RecordingPhase::Paused {
        return Err(YawrecError::InvalidState(
            "Aucun enregistrement en pause".into(),
        ));
    }
    s.phase = RecordingPhase::Recording;
    s.started_at = Some(Instant::now());
    s.audio_paused.store(false, Ordering::Relaxed);
    log::info!("⏵ resume_recording");
    Ok(())
}

#[tauri::command]
pub async fn recording_status(
    state: State<'_, Mutex<RecorderState>>,
) -> YawrecResult<StatusPayload> {
    let s = state.lock().unwrap();
    Ok(StatusPayload::from_state(&s))
}

// ============================================================
// Commandes : configuration
// ============================================================

#[tauri::command]
pub async fn set_capture_mode(
    mode: CaptureMode,
    state: State<'_, Mutex<RecorderState>>,
) -> YawrecResult<()> {
    let mut s = state.lock().unwrap();
    s.mode = mode;
    log::debug!("set_capture_mode → {:?}", mode);
    Ok(())
}

#[tauri::command]
pub async fn set_output_directory(
    path: String,
    state: State<'_, Mutex<RecorderState>>,
) -> YawrecResult<()> {
    let p = PathBuf::from(&path);
    if !p.is_dir() {
        return Err(YawrecError::Config(format!(
            "Le chemin n'est pas un dossier : {path}"
        )));
    }
    let mut s = state.lock().unwrap();
    s.output_dir = Some(p);
    log::debug!("set_output_directory → {}", path);
    Ok(())
}

#[tauri::command]
pub async fn set_screen_id(
    id: u32,
    state: State<'_, Mutex<RecorderState>>,
) -> YawrecResult<()> {
    let mut s = state.lock().unwrap();
    s.screen_id = Some(id);
    log::debug!("set_screen_id → {}", id);
    Ok(())
}

#[tauri::command]
pub async fn set_webcam_enabled(
    enabled: bool,
    state: State<'_, Mutex<RecorderState>>,
) -> YawrecResult<()> {
    let mut s = state.lock().unwrap();
    s.webcam_enabled = enabled;
    log::debug!("set_webcam_enabled → {}", enabled);
    Ok(())
}

#[tauri::command]
pub async fn set_webcam_id(
    id: u32,
    state: State<'_, Mutex<RecorderState>>,
) -> YawrecResult<()> {
    let mut s = state.lock().unwrap();
    s.webcam_device_id = Some(id);
    log::debug!("set_webcam_id → {}", id);
    Ok(())
}

// ============================================================
// Commandes : énumération
// ============================================================

#[tauri::command]
pub async fn list_audio_devices() -> YawrecResult<Vec<DeviceInfo>> {
    audio::list_devices().map_err(|e| YawrecError::Device(e.to_string()))
}

#[tauri::command]
pub async fn list_webcams() -> YawrecResult<Vec<DeviceInfo>> {
    webcam::list_devices().map_err(|e| YawrecError::Device(e.to_string()))
}

#[tauri::command]
pub async fn list_screens() -> YawrecResult<Vec<ScreenInfo>> {
    screen::list_screens().map_err(|e| YawrecError::Device(e.to_string()))
}

#[derive(Debug, Serialize, Clone)]
pub struct EncoderInfo {
    pub codec: String,
    pub display_name: String,
    pub hardware: bool,
}

#[tauri::command]
pub async fn get_active_encoder() -> YawrecResult<EncoderInfo> {
    let best = VideoEncoder::pick_best();
    Ok(EncoderInfo {
        codec: best.ffmpeg_name().to_string(),
        display_name: best.display_name().to_string(),
        hardware: best.is_hardware(),
    })
}

#[tauri::command]
pub async fn set_audio_config(
    mic_enabled: bool,
    loopback_enabled: bool,
    mic_device: Option<String>,
    state: State<'_, Mutex<RecorderState>>,
) -> YawrecResult<()> {
    // Mettre à jour l'état + arrêter l'ancien monitor en une seule prise de lock.
    let (phase, device_name, mic_level, mic_gain, new_stop) = {
        let mut s = state.lock().unwrap();
        s.mic_enabled     = mic_enabled;
        s.loopback_enabled = loopback_enabled;
        s.mic_device_name  = mic_device;
        // Signal + drop de l'ancien monitor
        s.mic_monitor_stop.store(true, Ordering::Relaxed);
        let _ = s.mic_monitor_handle.take();
        if !mic_enabled {
            s.mic_level.store(0f32.to_bits(), Ordering::Relaxed);
        }
        let new_stop = Arc::new(AtomicBool::new(false));
        let result = (
            s.phase,
            s.mic_device_name.clone(),
            Arc::clone(&s.mic_level),
            Arc::clone(&s.mic_gain),
            Arc::clone(&new_stop),
        );
        s.mic_monitor_stop = new_stop;
        result
    };

    log::debug!(
        "set_audio_config → mic={mic_enabled} loop={loopback_enabled} device={device_name:?}"
    );

    // Démarrer le nouveau monitor seulement si mic actif ET pas d'enregistrement en cours
    // (pendant l'enregistrement, le worker audio gère le niveau).
    if mic_enabled && phase == RecordingPhase::Idle {
        if let Ok(handle) = audio::start_monitor(
            device_name.as_deref(), mic_level, mic_gain, Arc::clone(&new_stop),
        ) {
            state.lock().unwrap().mic_monitor_handle = Some(handle);
        }
    }

    Ok(())
}

#[tauri::command]
pub async fn get_output_directory(
    state: State<'_, Mutex<RecorderState>>,
) -> YawrecResult<String> {
    let s = state.lock().unwrap();
    Ok(s.output_dir_display())
}

#[tauri::command]
pub async fn set_pip_position(
    position: PipPosition,
    state: State<'_, Mutex<RecorderState>>,
) -> YawrecResult<()> {
    let s = state.lock().unwrap();
    s.pip_position.store(position.to_u8(), Ordering::Relaxed);
    log::debug!("set_pip_position → {:?}", position);
    Ok(())
}

#[tauri::command]
pub async fn set_mic_gain(
    gain: f32,
    state: State<'_, Mutex<RecorderState>>,
) -> YawrecResult<()> {
    let gain = gain.clamp(0.0, 4.0);
    let s = state.lock().unwrap();
    s.mic_gain.store(gain.to_bits(), Ordering::Relaxed);
    log::debug!("set_mic_gain → {:.3} ({:.1} dB)", gain, 20.0 * gain.max(1e-6).log10());
    Ok(())
}

// ============================================================
// D6 — Capture fenêtre
// ============================================================

#[tauri::command]
pub async fn list_windows() -> YawrecResult<Vec<WindowInfo>> {
    Ok(window::list_windows())
}

#[tauri::command]
pub async fn set_window_hwnd(
    hwnd: i64,
    state: State<'_, Mutex<RecorderState>>,
) -> YawrecResult<()> {
    let mut s = state.lock().unwrap();
    s.selected_hwnd = if hwnd == 0 { None } else { Some(hwnd) };
    log::debug!("set_window_hwnd → {:?}", s.selected_hwnd);
    Ok(())
}
