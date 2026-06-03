// ============================================================
// YAWREC · ws_server.rs
// Serveur WebSocket local (port 9799) — contrôle à distance
// depuis l'app Android (ou n'importe quel client WS).
//
// Protocole (JSON) :
//   Phone → Bureau : {"cmd":"start|stop|pause|resume|status"}
//   Bureau → Phone : {"event":"status|stopped|error", ...}
// ============================================================

use std::net::SocketAddr;
use std::sync::atomic::Ordering;
use std::sync::Mutex;
use std::time::Instant;

use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::{accept_async, tungstenite::Message};

use crate::commands::StatusPayload;
use crate::state::{RecorderState, RecordingPhase};

pub const WS_PORT: u16 = 9799;

#[derive(Deserialize)]
struct WsCmd {
    cmd: String,
}

/// Événements envoyés au client Android.
/// `#[serde(tag = "event")]` produit : {"event":"status", <champs StatusPayload...>}
#[derive(Serialize)]
#[serde(tag = "event", rename_all = "snake_case")]
enum WsEvent {
    Status(StatusPayload),
    Stopped { path: String },
    Error { message: String },
}

// ── Point d'entrée : lance le listener, une goroutine par connexion ──────────

pub async fn run(app: AppHandle) {
    let addr = SocketAddr::from(([0, 0, 0, 0], WS_PORT));
    let listener = match TcpListener::bind(&addr).await {
        Ok(l) => l,
        Err(e) => {
            log::error!("WS remote server: bind :{WS_PORT} failed: {e}");
            return;
        }
    };
    log::info!("WS remote control server listening on :{WS_PORT}");

    loop {
        match listener.accept().await {
            Ok((stream, peer)) => {
                log::info!("WS: client connected from {peer}");
                let app = app.clone();
                tokio::spawn(async move {
                    if let Err(e) = handle(stream, app).await {
                        log::warn!("WS: client disconnected ({e})");
                    }
                });
            }
            Err(e) => log::warn!("WS: accept error: {e}"),
        }
    }
}

// ── Gestion d'un client ──────────────────────────────────────────────────────

async fn handle(stream: TcpStream, app: AppHandle) -> anyhow::Result<()> {
    let ws = accept_async(stream).await?;
    let (mut sink, mut source) = ws.split();

    // Statut initial immédiat
    sink.send(Message::Text(to_json(WsEvent::Status(get_status(&app))))).await?;

    // Canal interne : push_task + cmd_task → write loop
    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(32);

    // Push statut toutes les secondes
    let tx_push = tx.clone();
    let app_push = app.clone();
    let push_task = tokio::spawn(async move {
        let mut tick = tokio::time::interval(std::time::Duration::from_secs(1));
        loop {
            tick.tick().await;
            let json = to_json(WsEvent::Status(get_status(&app_push)));
            if tx_push.send(json).await.is_err() {
                break;
            }
        }
    });

    // Lecture des commandes
    let tx_cmd = tx.clone();
    let app_cmd = app.clone();
    let cmd_task = tokio::spawn(async move {
        while let Some(msg) = source.next().await {
            match msg {
                Ok(Message::Text(text)) => {
                    if let Ok(WsCmd { cmd }) = serde_json::from_str::<WsCmd>(&text) {
                        let resp = dispatch(&app_cmd, &cmd).await;
                        if tx_cmd.send(resp).await.is_err() {
                            break;
                        }
                    }
                }
                Ok(Message::Close(_)) | Err(_) => break,
                _ => {}
            }
        }
    });

    // Boucle d'écriture : vide le canal → sink WebSocket
    drop(tx);
    while let Some(msg) = rx.recv().await {
        if sink.send(Message::Text(msg)).await.is_err() {
            break;
        }
    }

    push_task.abort();
    cmd_task.abort();
    Ok(())
}

// ── Dispatch des commandes ───────────────────────────────────────────────────

async fn dispatch(app: &AppHandle, cmd: &str) -> String {
    match cmd {
        "start" => match crate::commands::do_start_recording(app).await {
            Ok(_)  => to_json(WsEvent::Status(get_status(app))),
            Err(e) => to_json(WsEvent::Error { message: e.to_string() }),
        },

        "stop" => match crate::commands::do_stop_recording(app).await {
            Ok(path) => to_json(WsEvent::Stopped { path }),
            Err(e)   => to_json(WsEvent::Error { message: e.to_string() }),
        },

        "pause" => {
            let state = app.state::<Mutex<RecorderState>>();
            let mut s = state.lock().unwrap();
            if s.phase != RecordingPhase::Recording {
                return to_json(WsEvent::Error { message: "Pas d'enregistrement actif".into() });
            }
            if let Some(t) = s.started_at.take() {
                s.paused_offset += t.elapsed();
            }
            s.phase = RecordingPhase::Paused;
            s.audio_paused.store(true, Ordering::Relaxed);
            drop(s);
            to_json(WsEvent::Status(get_status(app)))
        }

        "resume" => {
            let state = app.state::<Mutex<RecorderState>>();
            let mut s = state.lock().unwrap();
            if s.phase != RecordingPhase::Paused {
                return to_json(WsEvent::Error { message: "Aucun enregistrement en pause".into() });
            }
            s.phase = RecordingPhase::Recording;
            s.started_at = Some(Instant::now());
            s.audio_paused.store(false, Ordering::Relaxed);
            drop(s);
            to_json(WsEvent::Status(get_status(app)))
        }

        "status" => to_json(WsEvent::Status(get_status(app))),

        _ => to_json(WsEvent::Error { message: format!("Commande inconnue: {cmd}") }),
    }
}

// ── Helpers ──────────────────────────────────────────────────────────────────

fn get_status(app: &AppHandle) -> StatusPayload {
    let state = app.state::<Mutex<RecorderState>>();
    let s = state.lock().unwrap();
    StatusPayload::from_state(&s)
}

fn to_json(event: WsEvent) -> String {
    serde_json::to_string(&event)
        .unwrap_or_else(|_| r#"{"event":"error","message":"serialize failed"}"#.into())
}
