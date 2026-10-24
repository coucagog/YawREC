// ============================================================
// YAWREC · commands.rs
// Commandes Tauri (IPC) + workers + helpers partagés.
//
// Tous les points d'entrée (UI, raccourcis, tray, télécommande WebSocket)
// passent par les mêmes helpers do_start / do_stop / do_pause / do_resume,
// pour qu'un même geste ait partout le même effet.
//
// Temps : chaque enregistrement a un `recording_epoch` commun. Les workers
// vidéo et audio datent leurs données depuis cet epoch puis retirent les
// pauses via `PauseTimeline` — l'audio et la vidéo restent alignés quel que
// soit le chemin par lequel la pause a été déclenchée.
// ============================================================

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU64, AtomicU8, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde::Serialize;
use tauri::{AppHandle, Emitter, Manager, State};
use tauri_plugin_notification::NotificationExt;

use crate::capture::audio::AudioCapturer;
use crate::capture::webcam::PipLayout;
use crate::capture::window::WindowInfo;
use crate::capture::{audio, screen, webcam, window, Frame};
use crate::encoder::{Encoder, EncoderConfig, VideoEncoder};
use crate::error::{YawrecError, YawrecResult};
use crate::state::{CaptureMode, PauseTimeline, PipPosition, RecorderState, RecordingPhase};

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
    /// Coin haut-gauche en coordonnées bureau (pixels physiques).
    pub x: i32,
    pub y: i32,
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
fn pip_offset(
    screen_w: u32, screen_h: u32, pip_w: u32, pip_h: u32, margin: u32, pos: PipPosition,
) -> (usize, usize) {
    let m = margin as usize;
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
///   - si l'écran est trop petit pour le PiP et sa marge, on no-op
///   - opacité = 1.0 (pas d'alpha blending) — on overwrite simplement
fn composite_pip(
    screen: &mut [u8], screen_w: u32, screen_h: u32, pip: &Frame, margin: u32, pos: PipPosition,
) {
    if screen_w < pip.width + 2 * margin || screen_h < pip.height + 2 * margin {
        return;
    }

    let (x_off, y_off) = pip_offset(screen_w, screen_h, pip.width, pip.height, margin, pos);
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
}

// ============================================================
// D6 — Crop de frame pour les modes Window / Region
// ============================================================

/// Extrait un sous-rectangle (wx, wy, ww, wh) — coordonnées relatives à la
/// frame — d'une frame BGRA8. Les pixels hors limites restent noirs.
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
    epoch: Instant,
    pause_timeline: Arc<PauseTimeline>,
    byte_count: Arc<AtomicU64>,
    frame_count: Arc<AtomicU64>,
    encoder_arc: Arc<Mutex<Option<Encoder>>>,
    pip_buffer: Option<Arc<Mutex<Option<Frame>>>>,
    pip_position: Arc<AtomicU8>,
    pip_margin: u32,
    capture_mode: CaptureMode,
    selected_hwnd: Option<i64>,
    /// Dimensions fixes du crop en mode Window (déterminées au démarrage).
    crop_w: u32,
    crop_h: u32,
    /// Zone fixe du mode Region (x, y, w, h — coordonnées bureau).
    capture_region: Option<(i32, i32, u32, u32)>,
    video_fps: u32,
    video_bitrate_kbps: u32,
    app: AppHandle,
}

fn run_video_worker(ctx: VideoWorkerCtx) {
    log::info!(
        "Worker vidéo · démarrage (screen={}, mode={:?}, pip={}, crop={}×{}, {} fps)",
        ctx.screen_id, ctx.capture_mode, ctx.pip_buffer.is_some(), ctx.crop_w, ctx.crop_h, ctx.video_fps,
    );

    if let Some(msg) = video_loop(&ctx) {
        // Erreur fatale : on finalise ce qui a été enregistré et on remet
        // l'état à Idle, au lieu de laisser un timer tourner à vide.
        log::error!("Worker vidéo · {msg}");
        let _ = ctx.app.emit("recorder://error", &msg);
        stop_after_failure(ctx.app.clone());
    }

    log::info!("Worker vidéo · terminé (frames={})", ctx.frame_count.load(Ordering::Relaxed));
}

/// Boucle de capture. Retourne Some(message) en cas d'erreur fatale.
fn video_loop(ctx: &VideoWorkerCtx) -> Option<String> {
    use crate::capture::Capturer;

    let mut capturer = match screen::make_capturer(ctx.screen_id) {
        Ok(c) => c,
        Err(e) => return Some(format!("Capture écran : {e}")),
    };
    if let Err(e) = capturer.start() {
        return Some(format!("Démarrage capture : {e}"));
    }

    // Cadence : DXGI livre une image à chaque rafraîchissement qui change
    // l'écran (jusqu'à 144+ Hz) et rien quand il est statique. On encode à
    // la cadence demandée, en réutilisant la dernière image si besoin — ce
    // qui garde aussi la webcam animée sur un écran figé.
    let interval_ms = 1000.0 / ctx.video_fps.max(1) as f64;
    capturer.set_acquire_timeout_ms((interval_ms.ceil() as u32).clamp(5, 100));
    let mut next_due = 0.0f64;
    let mut last_base: Option<Frame> = None;
    let mut fatal = None;

    while !ctx.stop_flag.load(Ordering::Relaxed) {
        let fresh = match capturer.next_frame() {
            Ok(f) => f,
            Err(e) => { fatal = Some(format!("Capture frame : {e}")); break; }
        };

        if let Some(frame) = fresh {
            let (ox, oy) = capturer.desktop_origin();
            match ctx.capture_mode {
                CaptureMode::Fullscreen => last_base = Some(frame),
                CaptureMode::Window => {
                    // Position relue à chaque frame pour suivre les déplacements.
                    // Fenêtre minimisée ou fermée : on garde la dernière image.
                    if let Some((wx, wy, _, _)) = ctx.selected_hwnd.and_then(window::get_window_rect) {
                        last_base = Some(crop_frame_to_window(&frame, wx - ox, wy - oy, ctx.crop_w, ctx.crop_h));
                    }
                }
                CaptureMode::Region => {
                    if let Some((rx, ry, rw, rh)) = ctx.capture_region {
                        last_base = Some(crop_frame_to_window(&frame, rx - ox, ry - oy, rw, rh));
                    }
                }
            }
        }

        let wall_ms = ctx.epoch.elapsed().as_millis() as u64;
        let Some(ts_ms) = ctx.pause_timeline.media_ts(wall_ms) else { continue };
        let Some(base) = last_base.as_ref() else { continue };
        if (ts_ms as f64) + 1.0 < next_due {
            continue;
        }
        next_due += interval_ms;
        if next_due < ts_ms as f64 {
            next_due = ts_ms as f64 + interval_ms;
        }

        let mut out = base.data.clone();

        // D5 — composite PiP avant encodage
        if let Some(pip_arc) = &ctx.pip_buffer {
            if let Ok(guard) = pip_arc.lock() {
                if let Some(pip) = guard.as_ref() {
                    let pos = PipPosition::from_u8(ctx.pip_position.load(Ordering::Relaxed));
                    composite_pip(&mut out, base.width, base.height, pip, ctx.pip_margin, pos);
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
                // H.264 4:2:0 exige des dimensions paires.
                width: base.width & !1,
                height: base.height & !1,
                fps: ctx.video_fps,
                bitrate_kbps: ctx.video_bitrate_kbps,
                ..Default::default()
            };
            match Encoder::new(config) {
                Ok(e) => *guard = Some(e),
                Err(e) => { fatal = Some(format!("Encodeur : {e}")); break; }
            }
        }

        let enc = guard.as_mut().expect("encoder initialisé");
        if let Err(e) = enc.push_video_frame(&out, base.width, base.height, ts_ms) {
            fatal = Some(format!("Encodage vidéo : {e}"));
            break;
        }

        ctx.frame_count.fetch_add(1, Ordering::Relaxed);
        ctx.byte_count.store(enc.bytes_written(), Ordering::Relaxed);
    }

    if let Err(e) = capturer.stop() {
        log::warn!("capturer.stop() : {e}");
    }
    fatal
}

// ============================================================
// Worker audio
// ============================================================

struct AudioWorkerCtx {
    stop_flag: Arc<AtomicBool>,
    epoch: Instant,
    pause_timeline: Arc<PauseTimeline>,
    encoder_arc: Arc<Mutex<Option<Encoder>>>,
    mic_enabled: bool,
    loopback_enabled: bool,
    mic_device_name: Option<String>,
    mic_gain: Arc<AtomicU32>,
    mic_level: Arc<AtomicU32>,
    app: AppHandle,
}

fn run_audio_worker(ctx: AudioWorkerCtx) {
    if !ctx.mic_enabled && !ctx.loopback_enabled {
        log::info!("Worker audio · désactivé (mic et loopback OFF)");
        return;
    }

    log::info!("Worker audio · démarrage (mic={}, loopback={})", ctx.mic_enabled, ctx.loopback_enabled);

    let mut capturer = match AudioCapturer::open(
        ctx.mic_enabled,
        ctx.loopback_enabled,
        ctx.mic_device_name.as_deref(),
        ctx.mic_gain,
        ctx.mic_level,
        ctx.epoch,
    ) {
        Ok(c) => c,
        Err(e) => {
            log::warn!("Audio désactivé : {e}");
            notify(&ctx.app, "YawREC", &format!("Enregistrement sans audio : {e}"));
            return;
        }
    };

    log::info!("Audio sources : micro={}, loopback={}", capturer.has_mic(), capturer.has_loopback());

    // pull_chunk est cadencé sur l'horloge murale : on le sollicite souvent
    // et il n'émet que les chunks dus.
    while !ctx.stop_flag.load(Ordering::Relaxed) {
        std::thread::sleep(Duration::from_millis(10));

        while let Some((chunk, pos_ms)) = capturer.pull_chunk() {
            // Chunk tombant pendant une pause : jeté.
            let Some(ts_ms) = ctx.pause_timeline.media_ts(pos_ms) else { continue };

            let mut guard = match ctx.encoder_arc.lock() {
                Ok(g) => g,
                Err(p) => p.into_inner(),
            };
            // Pas encore d'encodeur (il naît avec la 1re frame vidéo) : jeté.
            if let Some(enc) = guard.as_mut() {
                if let Err(e) = enc.push_audio_samples(&chunk, ts_ms) {
                    log::error!("push_audio_samples : {e}");
                    return;
                }
            }
        }
    }

    log::info!("Worker audio · terminé");
}

// ============================================================
// Helpers partagés (IPC + raccourcis + tray + télécommande)
// ============================================================

/// Snapshot de la configuration nécessaire au démarrage.
struct StartConfig {
    mode: CaptureMode,
    screen_id: Option<u32>,
    selected_hwnd: Option<i64>,
    selected_region: Option<(i32, i32, u32, u32)>,
    webcam_enabled: bool,
}

/// Géométrie de capture résolue avant de lancer les workers.
struct CaptureGeometry {
    screen_id: u32,
    crop: (u32, u32),
    region: Option<(i32, i32, u32, u32)>,
    out_width: u32,
}

fn resolve_geometry(cfg: &StartConfig) -> YawrecResult<CaptureGeometry> {
    let screen_width = |id: u32| {
        screen::list_screens().ok()
            .and_then(|l| l.into_iter().find(|s| s.id == id))
            .map(|s| s.width)
            .unwrap_or(1920)
    };
    // L'écran capturé est celui qui contient le centre de la zone, pour que
    // fenêtre et région fonctionnent sur un écran secondaire.
    let screen_for = |x: i32, y: i32, w: u32, h: u32| {
        screen::screen_at(x + w as i32 / 2, y + h as i32 / 2)
            .map(|s| s.id)
            .or(cfg.screen_id)
            .unwrap_or(0)
    };

    match cfg.mode {
        CaptureMode::Fullscreen => {
            let id = cfg.screen_id.unwrap_or(0);
            Ok(CaptureGeometry { screen_id: id, crop: (0, 0), region: None, out_width: screen_width(id) })
        }
        CaptureMode::Window => {
            let (x, y, w, h) = cfg.selected_hwnd
                .and_then(window::get_window_rect)
                .ok_or_else(|| YawrecError::Config(
                    "Aucune fenêtre sélectionnée, ou la fenêtre est fermée / minimisée".into(),
                ))?;
            let crop = (w & !1, h & !1);
            if crop.0 < 2 || crop.1 < 2 {
                return Err(YawrecError::Config("Fenêtre trop petite".into()));
            }
            Ok(CaptureGeometry { screen_id: screen_for(x, y, w, h), crop, region: None, out_width: crop.0 })
        }
        CaptureMode::Region => {
            let (x, y, w, h) = cfg.selected_region
                .ok_or_else(|| YawrecError::Config("Aucune zone sélectionnée".into()))?;
            let (w, h) = (w & !1, h & !1);
            if w < 2 || h < 2 {
                return Err(YawrecError::Config("Zone trop petite".into()));
            }
            Ok(CaptureGeometry { screen_id: screen_for(x, y, w, h), crop: (0, 0), region: Some((x, y, w, h)), out_width: w })
        }
    }
}

pub async fn do_start_recording(app: &AppHandle) -> YawrecResult<()> {
    let state_mutex = app.state::<Mutex<RecorderState>>();

    // 1. Snapshot de la config (lock court), géométrie résolue hors lock
    //    (énumération DXGI, requêtes fenêtre).
    let cfg = {
        let s = state_mutex.lock().unwrap();
        if s.phase != RecordingPhase::Idle {
            return Err(YawrecError::InvalidState("Un enregistrement est déjà en cours".into()));
        }
        StartConfig {
            mode: s.mode,
            screen_id: s.screen_id,
            selected_hwnd: s.selected_hwnd,
            selected_region: s.selected_region,
            webcam_enabled: s.webcam_enabled,
        }
    };
    let geo = resolve_geometry(&cfg)?;
    let pip_layout = PipLayout::for_output_width(geo.out_width);

    // 2. Passage en Recording + préparation des Arcs pour les workers.
    let epoch = Instant::now();
    let (video_ctx, audio_ctx, webcam_job, mic_monitor_handle) = {
        let mut s = state_mutex.lock().unwrap();
        if s.phase != RecordingPhase::Idle {
            return Err(YawrecError::InvalidState("Un enregistrement est déjà en cours".into()));
        }
        s.phase = RecordingPhase::Recording;
        s.started_at = Some(epoch);
        s.paused_offset = Duration::ZERO;
        s.recording_epoch = Some(epoch);
        s.pause_timeline.reset();
        s.byte_count.store(0, Ordering::Relaxed);
        s.frame_count.store(0, Ordering::Relaxed);
        s.stop_flag.store(false, Ordering::Relaxed);

        let out = s.compute_output_path();
        s.current_output_path = Some(out.clone());

        let enc_arc: Arc<Mutex<Option<Encoder>>> = Arc::new(Mutex::new(None));
        s.encoder_arc = Some(Arc::clone(&enc_arc));

        // D5 — pip buffer si webcam activée
        let pip_buf = if cfg.webcam_enabled {
            let buf: Arc<Mutex<Option<Frame>>> = Arc::new(Mutex::new(None));
            s.pip_buffer = Some(Arc::clone(&buf));
            Some(buf)
        } else {
            s.pip_buffer = None;
            None
        };
        let webcam_job = pip_buf.clone().map(|b| (s.webcam_device_id.unwrap_or(0), b));

        let video_ctx = VideoWorkerCtx {
            screen_id: geo.screen_id,
            output_path: out,
            stop_flag: Arc::clone(&s.stop_flag),
            epoch,
            pause_timeline: Arc::clone(&s.pause_timeline),
            byte_count: Arc::clone(&s.byte_count),
            frame_count: Arc::clone(&s.frame_count),
            encoder_arc: Arc::clone(&enc_arc),
            pip_buffer: pip_buf,
            pip_position: Arc::clone(&s.pip_position),
            pip_margin: pip_layout.margin,
            capture_mode: cfg.mode,
            selected_hwnd: cfg.selected_hwnd,
            crop_w: geo.crop.0,
            crop_h: geo.crop.1,
            capture_region: geo.region,
            video_fps: s.video_fps,
            video_bitrate_kbps: s.video_bitrate_kbps,
            app: app.clone(),
        };
        let audio_ctx = AudioWorkerCtx {
            stop_flag: Arc::clone(&s.stop_flag),
            epoch,
            pause_timeline: Arc::clone(&s.pause_timeline),
            encoder_arc: enc_arc,
            mic_enabled: s.mic_enabled,
            loopback_enabled: s.loopback_enabled,
            mic_device_name: s.mic_device_name.clone(),
            mic_gain: Arc::clone(&s.mic_gain),
            mic_level: Arc::clone(&s.mic_level),
            app: app.clone(),
        };

        // Arrêt du monitor VU : le worker audio prend le relais.
        s.mic_monitor_stop.store(true, Ordering::Relaxed);
        (video_ctx, audio_ctx, webcam_job, s.mic_monitor_handle.take())
    };
    // Le thread monitor s'arrête seul (il vérifie son flag toutes les 100 ms).
    drop(mic_monitor_handle);

    let screen_id = video_ctx.screen_id;
    let has_webcam = webcam_job.is_some();
    let stop_flag = Arc::clone(&video_ctx.stop_flag);

    let spawned = (|| -> YawrecResult<_> {
        let video = std::thread::Builder::new()
            .name("yawrec-video-worker".into())
            .spawn(move || run_video_worker(video_ctx))
            .map_err(|e| YawrecError::Capture(format!("spawn vidéo : {e}")))?;
        let audio = std::thread::Builder::new()
            .name("yawrec-audio-worker".into())
            .spawn(move || run_audio_worker(audio_ctx))
            .map_err(|e| YawrecError::Capture(format!("spawn audio : {e}")))?;
        let webcam = match webcam_job {
            Some((idx, buf)) => {
                let stop = Arc::clone(&stop_flag);
                Some(std::thread::Builder::new()
                    .name("yawrec-webcam-worker".into())
                    .spawn(move || webcam::run_worker(idx, pip_layout, stop, buf))
                    .map_err(|e| YawrecError::Capture(format!("spawn webcam : {e}")))?)
            }
            None => None,
        };
        Ok((video, audio, webcam))
    })();

    match spawned {
        Ok((video, audio, webcam)) => {
            let mut s = state_mutex.lock().unwrap();
            s.video_worker  = Some(video);
            s.audio_worker  = Some(audio);
            s.webcam_worker = webcam;
        }
        Err(e) => {
            // Ne pas rester bloqué en Recording si un thread n'a pas pu naître.
            stop_flag.store(true, Ordering::Relaxed);
            let _ = do_stop_recording(app).await;
            return Err(e);
        }
    }

    // F3 — notification
    let webcam_label = if has_webcam { " · webcam ON" } else { "" };
    notify(app, "YawREC", &format!("● Enregistrement démarré{webcam_label}"));

    log::info!("▶ start_recording (mode={:?}, screen={screen_id}, webcam={has_webcam})", cfg.mode);
    Ok(())
}

pub async fn do_stop_recording(app: &AppHandle) -> YawrecResult<String> {
    let state_mutex = app.state::<Mutex<RecorderState>>();

    let (video_handle, audio_handle, webcam_handle, encoder_arc, fallback_path) = {
        let mut s = state_mutex.lock().unwrap();
        if s.phase == RecordingPhase::Idle {
            return Err(YawrecError::InvalidState("Aucun enregistrement en cours".into()));
        }
        s.phase = RecordingPhase::Idle;
        s.started_at = None;
        s.paused_offset = Duration::ZERO;
        s.recording_epoch = None;
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
    for (name, handle) in [("vidéo", video_handle), ("audio", audio_handle), ("webcam", webcam_handle)] {
        if let Some(h) = handle {
            match tokio::task::spawn_blocking(move || h.join()).await {
                Ok(Err(_)) => log::warn!("worker {name} : panic"),
                Err(e) => log::warn!("join worker {name} : {e}"),
                Ok(Ok(())) => {}
            }
        }
    }

    // Flush + close encoder
    let result: YawrecResult<String> = match encoder_arc {
        Some(enc_arc) => {
            let encoder = enc_arc.lock().unwrap_or_else(|p| p.into_inner()).take();
            match encoder {
                Some(mut enc) => match enc.stop() {
                    Ok(p) => Ok(p.to_string_lossy().to_string()),
                    Err(e) => {
                        log::error!("encoder.stop() : {e}");
                        Err(YawrecError::Encoding(format!("Finalisation du fichier : {e}")))
                    }
                },
                None => Err(YawrecError::Capture(
                    "Aucune frame capturée — enregistrement vide. Vérifiez la sélection et les permissions de capture.".into(),
                )),
            }
        }
        None => Ok(fallback_path),
    };

    // Toujours redémarrer le monitor VU, même si l'enregistrement a échoué.
    restart_mic_monitor(app);

    match result {
        Ok(final_path) => {
            log::info!("■ stop_recording → {}", final_path);
            let _ = app.emit("recorder://stopped", &final_path);
            notify(app, "YawREC · enregistrement terminé", &format!("Fichier : {final_path}"));
            Ok(final_path)
        }
        Err(e) => {
            log::warn!("■ stop_recording : {e}");
            let _ = app.emit("recorder://error", e.to_string());
            Err(e)
        }
    }
}

/// Relance le thread VU-mètre micro (hors enregistrement).
fn restart_mic_monitor(app: &AppHandle) {
    let state_mutex = app.state::<Mutex<RecorderState>>();
    let (mic_enabled, mic_device, mic_level, mic_gain, new_stop) = {
        let mut s = state_mutex.lock().unwrap();
        s.mic_level.store(0f32.to_bits(), Ordering::Relaxed);
        s.mic_monitor_stop.store(true, Ordering::Relaxed);
        let new_stop = Arc::new(AtomicBool::new(false));
        s.mic_monitor_stop = Arc::clone(&new_stop);
        (s.mic_enabled, s.mic_device_name.clone(), Arc::clone(&s.mic_level), Arc::clone(&s.mic_gain), new_stop)
    };
    if !mic_enabled {
        return;
    }
    if let Ok(handle) = audio::start_monitor(mic_device.as_deref(), mic_level, mic_gain, new_stop) {
        state_mutex.lock().unwrap().mic_monitor_handle = Some(handle);
    }
}

/// Après une erreur fatale d'un worker : finalise le fichier et repasse en Idle.
fn stop_after_failure(app: AppHandle) {
    tauri::async_runtime::spawn(async move {
        match do_stop_recording(&app).await {
            Ok(path) => log::info!("Enregistrement interrompu, fichier partiel conservé : {path}"),
            Err(YawrecError::InvalidState(_)) => {} // déjà arrêté par ailleurs
            Err(e) => log::warn!("Arrêt après erreur : {e}"),
        }
    });
}

pub fn do_pause_recording(app: &AppHandle) -> YawrecResult<()> {
    let state = app.state::<Mutex<RecorderState>>();
    let mut s = state.lock().unwrap();
    if s.phase != RecordingPhase::Recording {
        return Err(YawrecError::InvalidState("Pas d'enregistrement actif à mettre en pause".into()));
    }
    if let Some(t) = s.started_at.take() {
        s.paused_offset += t.elapsed();
    }
    let at = s.epoch_ms();
    s.pause_timeline.pause_at(at);
    s.phase = RecordingPhase::Paused;
    log::info!("⏸ pause ({at} ms)");
    Ok(())
}

pub fn do_resume_recording(app: &AppHandle) -> YawrecResult<()> {
    let state = app.state::<Mutex<RecorderState>>();
    let mut s = state.lock().unwrap();
    if s.phase != RecordingPhase::Paused {
        return Err(YawrecError::InvalidState("Aucun enregistrement en pause".into()));
    }
    let at = s.epoch_ms();
    s.pause_timeline.resume_at(at);
    s.phase = RecordingPhase::Recording;
    s.started_at = Some(Instant::now());
    log::info!("⏵ reprise ({at} ms)");
    Ok(())
}

/// Bascule pause/reprise (raccourci global).
pub fn do_toggle_pause(app: &AppHandle) {
    let phase = app.state::<Mutex<RecorderState>>().lock().unwrap().phase;
    let res = match phase {
        RecordingPhase::Recording => do_pause_recording(app),
        RecordingPhase::Paused => do_resume_recording(app),
        RecordingPhase::Idle => Ok(()),
    };
    if let Err(e) = res {
        log::warn!("toggle pause : {e}");
    }
}

pub async fn do_toggle_recording(app: AppHandle) {
    let phase = app.state::<Mutex<RecorderState>>().lock().unwrap().phase;
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
pub async fn start_recording(app: AppHandle) -> YawrecResult<()> {
    do_start_recording(&app).await
}

#[tauri::command]
pub async fn stop_recording(app: AppHandle) -> YawrecResult<String> {
    do_stop_recording(&app).await
}

#[tauri::command]
pub async fn pause_recording(app: AppHandle) -> YawrecResult<()> {
    do_pause_recording(&app)
}

#[tauri::command]
pub async fn resume_recording(app: AppHandle) -> YawrecResult<()> {
    do_resume_recording(&app)
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
    app: AppHandle,
    state: State<'_, Mutex<RecorderState>>,
) -> YawrecResult<()> {
    let phase = {
        let mut s = state.lock().unwrap();
        s.mic_enabled      = mic_enabled;
        s.loopback_enabled = loopback_enabled;
        s.mic_device_name  = mic_device;
        if !mic_enabled {
            s.mic_level.store(0f32.to_bits(), Ordering::Relaxed);
        }
        s.phase
    };

    log::debug!("set_audio_config → mic={mic_enabled} loop={loopback_enabled}");

    // Pendant l'enregistrement, le worker audio gère le niveau ; la nouvelle
    // config s'appliquera au prochain enregistrement.
    if phase == RecordingPhase::Idle {
        restart_mic_monitor(&app);
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

// ============================================================
// Ouvrir le fichier enregistré dans l'Explorateur Windows
// ============================================================

#[tauri::command]
pub async fn reveal_in_explorer(path: String) -> YawrecResult<()> {
    #[cfg(target_os = "windows")]
    std::process::Command::new("explorer")
        .arg(format!("/select,{}", path))
        .spawn()?;
    #[cfg(not(target_os = "windows"))]
    let _ = path;
    Ok(())
}

// ============================================================
// Mode Région — picker de zone
// ============================================================

/// `x`, `y` : coin haut-gauche en coordonnées bureau (pixels physiques).
#[tauri::command]
pub async fn set_region(
    x: i32,
    y: i32,
    w: u32,
    h: u32,
    state: State<'_, Mutex<RecorderState>>,
    app: AppHandle,
) -> YawrecResult<()> {
    {
        let mut s = state.lock().unwrap();
        s.selected_region = if w > 0 && h > 0 { Some((x, y, w, h)) } else { None };
        log::debug!("set_region → ({x},{y},{w}×{h})");
    }
    // Notifie la fenêtre principale pour qu'elle mette à jour le label.
    let _ = app.emit("recorder://region-set", serde_json::json!({ "x": x, "y": y, "w": w, "h": h }));
    Ok(())
}

// ============================================================
// Préfixe du nom de fichier
// ============================================================

#[tauri::command]
pub async fn set_filename_prefix(
    prefix: String,
    state: State<'_, Mutex<RecorderState>>,
) -> YawrecResult<()> {
    let clean: String = prefix
        .chars()
        .filter(|c| c.is_alphanumeric() || matches!(*c, '-' | '_' | ' ' | '.'))
        .take(40)
        .collect();
    let mut s = state.lock().unwrap();
    s.filename_prefix = clean;
    Ok(())
}

// ============================================================
// Qualité vidéo — fps + bitrate
// ============================================================

#[derive(Debug, serde::Serialize)]
pub struct VideoQualityInfo {
    pub fps: u32,
    pub bitrate_kbps: u32,
}

#[tauri::command]
pub async fn set_video_quality(
    fps: u32,
    bitrate_kbps: u32,
    state: State<'_, Mutex<RecorderState>>,
) -> YawrecResult<()> {
    let mut s = state.lock().unwrap();
    s.video_fps           = fps.clamp(1, 120);
    s.video_bitrate_kbps  = bitrate_kbps.clamp(500, 100_000);
    log::debug!("set_video_quality → {}fps / {}kbps", s.video_fps, s.video_bitrate_kbps);
    Ok(())
}

#[tauri::command]
pub async fn get_video_quality(
    state: State<'_, Mutex<RecorderState>>,
) -> YawrecResult<VideoQualityInfo> {
    let s = state.lock().unwrap();
    Ok(VideoQualityInfo { fps: s.video_fps, bitrate_kbps: s.video_bitrate_kbps })
}

#[tauri::command]
pub async fn open_region_picker(app: AppHandle) -> YawrecResult<()> {
    if let Some(w) = app.get_webview_window("region-picker") {
        let _ = w.show();
        let _ = w.set_focus();
        return Ok(());
    }
    let mut builder = tauri::WebviewWindowBuilder::new(
        &app,
        "region-picker",
        tauri::WebviewUrl::App("region-picker.html".into()),
    );
    // Ouvrir le picker sur l'écran où se trouve la souris (sinon il
    // s'ouvre toujours sur l'écran principal).
    let monitor = app.cursor_position().ok()
        .and_then(|p| app.monitor_from_point(p.x, p.y).ok().flatten());
    if let Some(m) = monitor {
        let pos = m.position().to_logical::<f64>(m.scale_factor());
        builder = builder.position(pos.x, pos.y);
    }
    builder
    .title("YawREC — Sélection région")
    .decorations(false)
    .transparent(true)
    .fullscreen(true)
    .always_on_top(true)
    .skip_taskbar(true)
    .build()
    .map_err(|e| YawrecError::Config(format!("picker : {e}")))?;
    Ok(())
}

#[tauri::command]
pub fn get_local_ip() -> String {
    crate::ws_server::local_ip()
}

// ============================================================
// Tests
// ============================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(w: u32, h: u32, fill: impl Fn(u32, u32) -> [u8; 4]) -> Frame {
        let mut data = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                data.extend_from_slice(&fill(x, y));
            }
        }
        Frame { width: w, height: h, stride: w * 4, data, timestamp: Duration::ZERO }
    }

    #[test]
    fn human_size_unites() {
        assert_eq!(human_size(512), "512 B");
        assert_eq!(human_size(2048), "2 KB");
        assert_eq!(human_size(5 * 1024 * 1024), "5.00 MB");
        assert_eq!(human_size(3 * 1024 * 1024 * 1024), "3.00 GB");
    }

    #[test]
    fn pip_offset_coins() {
        assert_eq!(pip_offset(100, 80, 20, 10, 5, PipPosition::TopLeft), (5, 5));
        assert_eq!(pip_offset(100, 80, 20, 10, 5, PipPosition::TopRight), (75, 5));
        assert_eq!(pip_offset(100, 80, 20, 10, 5, PipPosition::BottomLeft), (5, 65));
        assert_eq!(pip_offset(100, 80, 20, 10, 5, PipPosition::BottomRight), (75, 65));
    }

    #[test]
    fn composite_pip_ecrit_dans_le_coin() {
        let mut screen = frame(10, 10, |_, _| [0, 0, 0, 255]);
        let pip = frame(2, 2, |_, _| [9, 9, 9, 255]);
        composite_pip(&mut screen.data, 10, 10, &pip, 1, PipPosition::BottomRight);
        let px = |x: u32, y: u32| screen.data[((y * 10 + x) * 4) as usize];
        assert_eq!(px(7, 7), 9);
        assert_eq!(px(8, 8), 9);
        assert_eq!(px(9, 9), 0, "marge respectée");
        assert_eq!(px(6, 7), 0);
    }

    #[test]
    fn composite_pip_trop_grand_ignore() {
        let mut screen = frame(4, 4, |_, _| [0, 0, 0, 255]);
        let pip = frame(4, 4, |_, _| [9, 9, 9, 255]);
        composite_pip(&mut screen.data, 4, 4, &pip, 1, PipPosition::TopLeft);
        assert!(screen.data.chunks_exact(4).all(|p| p[0] == 0));
    }

    #[test]
    fn crop_avec_debordement_remplit_de_noir() {
        let src = frame(4, 4, |x, y| [(x + 10 * y) as u8, 0, 0, 255]);
        // Zone 3×3 débordant en haut à gauche de 1 px.
        let out = crop_frame_to_window(&src, -1, -1, 3, 3);
        assert_eq!((out.width, out.height), (3, 3));
        let px = |x: u32, y: u32| out.data[((y * 3 + x) * 4) as usize];
        assert_eq!(out.data[3], 0, "coin hors source = noir (alpha 0)");
        assert_eq!(px(1, 1), 0);   // source (0,0)
        assert_eq!(px(2, 2), 11);  // source (1,1)
        assert_eq!(px(2, 1), 1);   // source (1,0)
    }

    #[test]
    fn crop_entierement_hors_source() {
        let src = frame(4, 4, |_, _| [7, 7, 7, 255]);
        let out = crop_frame_to_window(&src, 10, 10, 2, 2);
        assert!(out.data.iter().all(|&b| b == 0));
    }
}
