// ============================================================
// YAWREC · lib.rs
// Point d'entrée Tauri.
//
// Initialisation :
//   - plugins (dialog, notification, global-shortcut)
//   - état partagé (RecorderState dans un Mutex)
//   - probe encodeur HW (E2)
//   - tick loop 250 ms (C3)
//   - F1 : raccourci global Ctrl+Shift+R (toggle recording)
//   - F4 : tray icon avec menu (Afficher / Démarrer-Arrêter / Quitter)
// ============================================================

mod commands;
mod state;
mod error;
mod capture;
mod encoder;

use std::sync::Mutex;
use std::time::Duration;

use tauri::{
    menu::{Menu, MenuItem, PredefinedMenuItem},
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Emitter, Manager,
};
use tauri_plugin_global_shortcut::{Code, GlobalShortcutExt, Modifiers, Shortcut, ShortcutState};

use crate::commands::{do_pause_recording, do_toggle_recording, StatusPayload};
use crate::state::{RecorderState, RecordingPhase};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    // Logs vers %TEMP%\yawrec.log — toujours actif (release + debug).
    // Consulter ce fichier pour diagnostiquer les problèmes de capture.
    let log_path = std::env::temp_dir().join("yawrec.log");
    let log_file = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .open(&log_path)
        .ok();

    let mut log_builder = env_logger::Builder::from_env(
        env_logger::Env::default().default_filter_or("info,yawrec=debug"),
    );
    if let Some(f) = log_file {
        log_builder.target(env_logger::Target::Pipe(Box::new(f)));
    }
    log_builder.init();

    log::info!("YawREC · démarrage");

    // Ctrl+Shift+R — toggle enregistrement
    let ctrl_shift_r = Shortcut::new(
        Some(Modifiers::CONTROL | Modifiers::SHIFT),
        Code::KeyR,
    );
    // Ctrl+Shift+P — pause/reprendre
    let ctrl_shift_p = Shortcut::new(
        Some(Modifiers::CONTROL | Modifiers::SHIFT),
        Code::KeyP,
    );

    tauri::Builder::default()
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
                    if shortcut == &ctrl_shift_r {
                        log::debug!("Ctrl+Shift+R déclenché");
                        let app = app.clone();
                        tauri::async_runtime::spawn(async move {
                            do_toggle_recording(app).await;
                        });
                    } else if shortcut == &ctrl_shift_p {
                        log::debug!("Ctrl+Shift+P déclenché");
                        let app = app.clone();
                        tauri::async_runtime::spawn(async move {
                            do_pause_recording(app).await;
                        });
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
        ])
        .setup(move |app| {
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
            if let Err(e) = app.global_shortcut().register(ctrl_shift_r) {
                log::warn!("raccourci Ctrl+Shift+R impossible : {e}");
            } else {
                log::info!("Ctrl+Shift+R actif");
            }
            if let Err(e) = app.global_shortcut().register(ctrl_shift_p) {
                log::warn!("raccourci Ctrl+Shift+P impossible : {e}");
            } else {
                log::info!("Ctrl+Shift+P actif");
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

            log::info!("YawREC · fenêtre prête");
            Ok(())
        })
        .run(tauri::generate_context!())
        .expect("YawREC : erreur fatale au lancement");
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
        app, "tray-toggle", "Démarrer / Arrêter (Ctrl+Shift+R)", true, None::<&str>,
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
            "tray-show" => {
                if let Some(w) = app.get_webview_window("main") {
                    let _ = w.show();
                    let _ = w.set_focus();
                    let _ = w.unminimize();
                }
            }
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
                if let Some(w) = tray.app_handle().get_webview_window("main") {
                    let _ = w.show();
                    let _ = w.set_focus();
                    let _ = w.unminimize();
                }
            }
        })
        .build(app)?;

    log::info!("F4 · tray icon active");
    Ok(())
}
