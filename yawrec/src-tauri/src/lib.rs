// ============================================================
// YAWREC · lib.rs
// Point d'entrée Tauri.
//
// Initialisation :
//   - plugins (single-instance, dialog, notification, global-shortcut)
//   - état partagé (RecorderState dans un Mutex)
//   - probe encodeur HW (E2)
//   - tick loop 250 ms (C3)
//   - F1 : raccourcis globaux Ctrl+Shift+F9 (start/stop) et Ctrl+Shift+F10 (pause)
//   - F4 : tray icon avec menu (Afficher / Démarrer-Arrêter / Quitter)
//   - fermeture/quitter pendant un enregistrement : le MP4 est finalisé
//     avant la sortie (sinon l'index moov manque et le fichier est illisible)
// ============================================================

mod commands;
mod state;
mod error;
mod capture;
mod encoder;
mod ws_server;

use std::io::Write;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Mutex;
use std::time::Duration;

use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Emitter, Manager, RunEvent,
};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

use crate::commands::{do_stop_recording, do_toggle_pause, do_toggle_recording, StatusPayload};
use crate::state::{RecorderState, RecordingPhase};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Logs vers %TEMP%\yawrec.log — toujours actif (release + debug).
    // Consulter ce fichier pour diagnostiquer les problèmes de capture.
    // Le fichier n'est ouvert qu'une fois l'instance unique confirmée
    // (voir `open_log_file` dans setup).
    env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("info,yawrec=debug"),
    )
    .target(env_logger::Target::Pipe(Box::new(LogSink)))
    .init();

    log::info!("YawREC · démarrage");

    // Raccourcis globaux. Ctrl+Shift+R / Ctrl+Shift+P étaient captés dans
    // tout le système et volaient le rechargement forcé des navigateurs et
    // la palette de commandes de VS Code ; les touches F9/F10 avec
    // Ctrl+Shift sont très peu utilisées.
    // Ctrl+Shift+F9 — toggle enregistrement
    let shortcut_record = Shortcut::new(
        Some(Modifiers::CONTROL | Modifiers::SHIFT),
        Code::F9,
    );
    // Ctrl+Shift+F10 — pause/reprendre
    let shortcut_pause = Shortcut::new(
        Some(Modifiers::CONTROL | Modifiers::SHIFT),
        Code::F10,
    );

    tauri::Builder::default()
        // ----------------------------------------------------
        // Instance unique — doit être le premier plugin. Une 2e ouverture
        // (double-clic, raccourci du menu Démarrer) ramène la fenêtre
        // existante ; sans ça elle démarrait en tâche de fond, sans
        // raccourcis globaux ni serveur WebSocket (déjà pris par la 1re).
        // ----------------------------------------------------
        .plugin(tauri_plugin_single_instance::init(|app, _args, _cwd| {
            log::info!("2e instance lancée — fenêtre existante ramenée");
            show_main_window(app);
        }))
        .plugin(tauri_plugin_dialog::init())
        .plugin(tauri_plugin_notification::init())
        // ----------------------------------------------------
        // F1 : global shortcut. Le handler reçoit TOUS les raccourcis
        // enregistrés ; on filtre sur le nôtre puis on spawn la logique
        // async pour ne pas bloquer le thread d'événements.
        // ----------------------------------------------------
        .plugin(
            tauri_plugin_global_shortcut::Builder::new()
                .with_handler(move |app, shortcut, event| {
                    if event.state() != ShortcutState::Pressed {
                        return;
                    }
                    if shortcut == &shortcut_record {
                        log::debug!("Ctrl+Shift+F9 déclenché");
                        let app = app.clone();
                        tauri::async_runtime::spawn(async move {
                            do_toggle_recording(app).await;
                        });
                    } else if shortcut == &shortcut_pause {
                        log::debug!("Ctrl+Shift+F10 déclenché");
                        do_toggle_pause(app);
                    }
                })
                .build(),
        )
        .manage(Mutex::new(RecorderState::default()))
        .invoke_handler(tauri::generate_handler![
            commands::start_recording,
            commands::stop_recording,
            commands::pause_recording,
            commands::resume_recording,
            commands::recording_status,
            commands::list_audio_devices,
            commands::list_webcams,
            commands::list_screens,
            commands::set_capture_mode,
            commands::set_output_directory,
            commands::set_screen_id,
            commands::set_webcam_enabled,
            commands::set_webcam_id,
            commands::get_active_encoder,
            commands::set_audio_config,
            commands::get_output_directory,
            commands::set_pip_position,
            commands::set_mic_gain,
            commands::set_loopback_delay,
            commands::list_windows,
            commands::set_window_hwnd,
            commands::reveal_in_explorer,
            commands::set_region,
            commands::open_region_picker,
            commands::set_video_quality,
            commands::get_video_quality,
            commands::set_filename_prefix,
            commands::get_local_ip,
            ws_server::get_remote_pairing,
        ])
        .setup(move |app| {
            // Le plugin single-instance a déjà fait quitter une éventuelle
            // 2e instance : on peut vider et ouvrir le journal.
            open_log_file();

            // ----------------------------------------------------
            // E2 : probe encodeur en arrière-plan pour ne pas bloquer
            // le démarrage si un driver HW est lent ou défaillant.
            // ----------------------------------------------------
            tauri::async_runtime::spawn_blocking(|| {
                let best = crate::encoder::VideoEncoder::pick_best();
                log::info!("Encodeur prêt : {}", best.ffmpeg_name());
            });

            // ----------------------------------------------------
            // F1 : enregistrer le raccourci
            // ----------------------------------------------------
            for (shortcut, label) in [(shortcut_record, "Ctrl+Shift+F9"), (shortcut_pause, "Ctrl+Shift+F10")] {
                match app.global_shortcut().register(shortcut) {
                    Ok(()) => log::info!("{label} actif"),
                    Err(e) => log::warn!("raccourci {label} impossible : {e}"),
                }
            }

            // ----------------------------------------------------
            // F4 : tray icon
            // ----------------------------------------------------
            if let Err(e) = setup_tray(app.handle()) {
                log::warn!("F4 · tray icon désactivé : {e}");
            }

            // ----------------------------------------------------
            // Tick loop (C3)
            // ----------------------------------------------------
            let app_handle = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let mut interval = tokio::time::interval(Duration::from_millis(250));
                loop {
                    interval.tick().await;
                    let payload_opt = {
                        let state_mutex = app_handle.state::<Mutex<RecorderState>>();
                        let s = state_mutex.lock().unwrap();
                        if s.phase == RecordingPhase::Idle {
                            None
                        } else {
                            Some(StatusPayload::from_state(&s))
                        }
                    };
                    if let Some(payload) = payload_opt {
                        if let Err(e) = app_handle.emit("recorder://tick", &payload) {
                            log::warn!("emit tick failed: {e}");
                        }
                    }
                }
            });

            // ----------------------------------------------------
            // VU-mètre : émet le niveau micro à 100 ms (toujours actif)
            // ----------------------------------------------------
            let app_vu = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                let mut interval = tokio::time::interval(Duration::from_millis(100));
                loop {
                    interval.tick().await;
                    let level = {
                        let state_mutex = app_vu.state::<Mutex<RecorderState>>();
                        let s = state_mutex.lock().unwrap();
                        f32::from_bits(s.mic_level.load(std::sync::atomic::Ordering::Relaxed))
                    };
                    let _ = app_vu.emit("recorder://vu", level);
                }
            });

            // ----------------------------------------------------
            // Serveur WebSocket — contrôle à distance Android
            // ----------------------------------------------------
            let token = crate::ws_server::load_or_create_token(app.handle());
            app.manage(crate::ws_server::RemoteToken(token));
            let app_ws = app.handle().clone();
            tauri::async_runtime::spawn(async move {
                crate::ws_server::run(app_ws).await;
            });

            log::info!("YawREC · fenêtre prête");
            Ok(())
        })
        .build(tauri::generate_context!())
        .expect("YawREC : erreur fatale au lancement")
        .run(|app, event| {
            if let RunEvent::ExitRequested { api, .. } = event {
                // Sortie (fenêtre fermée, « Quitter » du tray…) pendant un
                // enregistrement : on retient la sortie le temps de finaliser
                // le MP4, puis on quitte pour de bon.
                static FINALIZING: AtomicBool = AtomicBool::new(false);
                let recording = app.state::<Mutex<RecorderState>>().lock().unwrap().phase
                    != RecordingPhase::Idle;
                if !recording {
                    return;
                }
                // Toute demande de sortie pendant la finalisation est retenue ;
                // la sortie finale a lieu une fois l'état repassé à Idle.
                api.prevent_exit();
                if !FINALIZING.swap(true, Ordering::SeqCst) {
                    log::info!("Sortie demandée pendant l'enregistrement — finalisation du fichier");
                    let app = app.clone();
                    tauri::async_runtime::spawn(async move {
                        if let Err(e) = do_stop_recording(&app).await {
                            log::warn!("finalisation avant sortie : {e}");
                        }
                        app.exit(0);
                    });
                }
            }
        });
}

// ============================================================
// Journal %TEMP%\yawrec.log
// ============================================================

/// État du journal : lignes en attente tant que le fichier n'est pas ouvert,
/// puis le fichier (ou rien : repli sur stderr s'il n'a pas pu être ouvert).
struct LogState {
    pending: Option<Vec<u8>>,
    file: Option<std::fs::File>,
}

static LOG_STATE: Mutex<LogState> = Mutex::new(LogState { pending: Some(Vec::new()), file: None });

/// Sortie d'env_logger. Ouvrir (et vider) le fichier dès le lancement
/// effaçait le journal de l'instance en cours quand on relançait l'app :
/// la 2e instance le tronquait avant d'être fermée par single-instance.
/// Les lignes émises avant `open_log_file` sont gardées en mémoire.
struct LogSink;

impl Write for LogSink {
    fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
        let mut st = LOG_STATE.lock().unwrap_or_else(|p| p.into_inner());
        let LogState { pending, file } = &mut *st;
        if let Some(f) = file {
            f.write(buf)
        } else if let Some(p) = pending {
            p.extend_from_slice(buf);
            Ok(buf.len())
        } else {
            std::io::stderr().write(buf)
        }
    }

    fn flush(&mut self) -> std::io::Result<()> {
        match &mut LOG_STATE.lock().unwrap_or_else(|p| p.into_inner()).file {
            Some(f) => f.flush(),
            None => Ok(()),
        }
    }
}

/// Ouvre %TEMP%\yawrec.log (vidé) et y recopie les lignes en attente.
fn open_log_file() {
    let path = std::env::temp_dir().join("yawrec.log");
    let opened = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&path);
    let mut st = LOG_STATE.lock().unwrap_or_else(|p| p.into_inner());
    let pending = st.pending.take().unwrap_or_default();
    match opened {
        Ok(mut f) => {
            let _ = f.write_all(&pending);
            st.file = Some(f);
        }
        Err(e) => {
            let _ = std::io::stderr().write_all(&pending);
            drop(st);
            log::warn!("Journal {} indisponible : {e}", path.display());
        }
    }
}

/// Affiche la fenêtre principale au premier plan (tray, 2e instance).
fn show_main_window(app: &tauri::AppHandle) {
    if let Some(w) = app.get_webview_window("main") {
        let _ = w.show();
        let _ = w.unminimize();
        let _ = w.set_focus();
    }
}

// ============================================================
// F4 — Tray icon
// ============================================================

fn setup_tray(app: &tauri::AppHandle) -> tauri::Result<()> {
    // Items du menu
    let show_item = MenuItem::with_id(
        app, "tray-show", "Afficher YawREC", true, None::<&str>,
    )?;
    let toggle_item = MenuItem::with_id(
        app, "tray-toggle", "Démarrer / Arrêter (Ctrl+Shift+F9)", true, None::<&str>,
    )?;
    let separator = PredefinedMenuItem::separator(app)?;
    let quit_item = MenuItem::with_id(
        app, "tray-quit", "Quitter YawREC", true, None::<&str>,
    )?;
    let menu = Menu::with_items(
        app,
        &[&show_item, &toggle_item, &separator, &quit_item],
    )?;

    // L'icône : on prend celle de la fenêtre principale. Si pas d'icône
    // configurée dans tauri.conf.json (icons/ vide en dev), on log et on
    // abandonne — le reste de l'app continue de fonctionner.
    let icon = match app.default_window_icon() {
        Some(i) => i.clone(),
        None => {
            return Err(tauri::Error::Anyhow(anyhow::anyhow!(
                "Aucune icône d'application — placer un icon.ico dans src-tauri/icons/"
            )));
        }
    };

    let _tray = TrayIconBuilder::with_id("yawrec-tray")
        .icon(icon)
        .menu(&menu)
        .show_menu_on_left_click(false)
        .tooltip("YawREC")
        .on_menu_event(|app, event| match event.id.as_ref() {
            "tray-show" => show_main_window(app),
            "tray-toggle" => {
                let app = app.clone();
                tauri::async_runtime::spawn(async move {
                    do_toggle_recording(app).await;
                });
            }
            "tray-quit" => {
                log::info!("F4 · sortie via tray");
                app.exit(0);
            }
            other => log::trace!("tray menu event inconnu : {other}"),
        })
        .on_tray_icon_event(|tray, event| {
            // Clic gauche sur l'icône → ramène la fenêtre devant.
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                ..
            } = event
            {
                show_main_window(tray.app_handle());
            }
        })
        .build(app)?;

    log::info!("F4 · tray icon active");
    Ok(())
}
