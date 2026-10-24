// ============================================================
// YAWREC · ws_server.rs
// Serveur WebSocket local (port 9799) — contrôle à distance
// depuis l'app Android (ou n'importe quel client WS).
//
// Sécurité :
//   - un jeton d'appairage (affiché dans le QR code du bureau) est exigé
//     dans l'URL : ws://<ip>:9799/?token=XXXX — sans lui, n'importe quel
//     appareil du Wi-Fi pouvait lancer un enregistrement ;
//   - toute requête portant un en-tête Origin est refusée : seuls les
//     navigateurs l'envoient, ce qui empêche une page web ouverte sur le PC
//     de piloter l'enregistreur via ws://127.0.0.1:9799.
//
// Protocole (JSON) :
//   Phone → Bureau : {"cmd":"start|stop|pause|resume|status"}
//   Bureau → Phone : {"event":"status|stopped|error", ...}
// ============================================================

use std::net::SocketAddr;
use std::sync::Mutex;
use std::time::Duration;

use futures_util::{SinkExt, StreamExt};
use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Manager};
use tokio::net::{TcpListener, TcpStream};
use tokio_tungstenite::{
    accept_hdr_async,
    tungstenite::{
        handshake::server::{ErrorResponse, Request, Response},
        http::StatusCode,
        Message,
    },
};

use crate::commands::StatusPayload;
use crate::state::RecorderState;

pub const WS_PORT: u16 = 9799;
const TOKEN_LEN: usize = 10;
const TOKEN_ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";

/// Jeton d'appairage de la télécommande (état Tauri managé).
pub struct RemoteToken(pub String);

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

// ── Jeton d'appairage ─────────────────────────────────────────────────────────

fn generate_token() -> String {
    use std::collections::hash_map::RandomState;
    use std::hash::{BuildHasher, Hasher};

    // RandomState est initialisé depuis l'aléa de l'OS : suffisant pour un
    // jeton LAN de 50 bits sans ajouter de dépendance.
    let mut out = String::with_capacity(TOKEN_LEN);
    let mut round = 0u64;
    while out.len() < TOKEN_LEN {
        let mut h = RandomState::new().build_hasher();
        h.write_u64(round);
        h.write_u128(std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or_default());
        let mut v = h.finish();
        for _ in 0..(64 / 5) {
            if out.len() == TOKEN_LEN {
                break;
            }
            out.push(TOKEN_ALPHABET[(v & 31) as usize] as char);
            v >>= 5;
        }
        round += 1;
    }
    out
}

fn is_valid_token(t: &str) -> bool {
    t.len() == TOKEN_LEN && t.bytes().all(|b| TOKEN_ALPHABET.contains(&b))
}

/// Charge le jeton depuis le dossier de config de l'app, ou en crée un.
/// Persistant : le téléphone n'a pas à rescanner le QR à chaque lancement.
pub fn load_or_create_token(app: &AppHandle) -> String {
    let path = app.path().app_config_dir().ok().map(|d| d.join("remote_token"));
    if let Some(p) = &path {
        if let Ok(t) = std::fs::read_to_string(p) {
            let t = t.trim().to_string();
            if is_valid_token(&t) {
                return t;
            }
        }
    }
    let token = generate_token();
    if let Some(p) = &path {
        let saved = p.parent().map_or(Ok(()), std::fs::create_dir_all)
            .and_then(|_| std::fs::write(p, &token));
        if let Err(e) = saved {
            log::warn!("WS: impossible d'enregistrer le jeton ({e}) — il changera au prochain lancement");
        }
    }
    token
}

/// Comparaison en temps constant (évite de deviner le jeton caractère par caractère).
fn token_matches(given: &str, expected: &str) -> bool {
    given.len() == expected.len()
        && given.bytes().zip(expected.bytes()).fold(0u8, |acc, (a, b)| acc | (a ^ b)) == 0
}

fn query_param<'a>(query: &'a str, key: &str) -> Option<&'a str> {
    query.split('&').find_map(|kv| {
        let (k, v) = kv.split_once('=')?;
        (k == key).then_some(v)
    })
}

/// Validation du handshake : jeton correct et pas d'en-tête Origin.
fn check_handshake(req: &Request, expected: &str) -> Result<(), (StatusCode, &'static str)> {
    if req.headers().contains_key("origin") {
        return Err((StatusCode::FORBIDDEN, "Connexions depuis un navigateur refusées"));
    }
    let token = req.uri().query().and_then(|q| query_param(q, "token")).unwrap_or("");
    if !token_matches(token, expected) {
        return Err((StatusCode::UNAUTHORIZED, "Jeton d'appairage invalide"));
    }
    Ok(())
}

// ── Adresse locale / appairage ───────────────────────────────────────────────

/// IP locale utilisée pour sortir vers Internet (sans rien envoyer : un
/// socket UDP "connecté" suffit à choisir l'interface).
pub fn local_ip() -> String {
    use std::net::UdpSocket;
    UdpSocket::bind("0.0.0.0:0")
        .and_then(|s| { s.connect("8.8.8.8:80")?; s.local_addr() })
        .map(|a| a.ip().to_string())
        .unwrap_or_else(|_| "127.0.0.1".to_string())
}

#[derive(Debug, Serialize)]
pub struct RemotePairing {
    pub ip: String,
    pub port: u16,
    pub token: String,
    /// Contenu du QR code : yawrec://<ip>:<port>?token=<token>
    pub url: String,
}

#[tauri::command]
pub fn get_remote_pairing(app: AppHandle) -> RemotePairing {
    let ip = local_ip();
    let token = app.state::<RemoteToken>().0.clone();
    RemotePairing {
        url: format!("yawrec://{ip}:{WS_PORT}?token={token}"),
        ip,
        port: WS_PORT,
        token,
    }
}

// ── Point d'entrée : lance le listener, une tâche par connexion ──────────────

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
                let app = app.clone();
                tokio::spawn(async move {
                    match handle(stream, app).await {
                        Ok(()) => log::info!("WS: client {peer} déconnecté"),
                        Err(e) => log::warn!("WS: client {peer} : {e}"),
                    }
                });
            }
            Err(e) => log::warn!("WS: accept error: {e}"),
        }
    }
}

// ── Gestion d'un client ──────────────────────────────────────────────────────

async fn handle(stream: TcpStream, app: AppHandle) -> anyhow::Result<()> {
    let expected = app.state::<RemoteToken>().0.clone();
    let ws = match accept_hdr_async(stream, |req: &Request, resp: Response| {
        match check_handshake(req, &expected) {
            Ok(()) => Ok(resp),
            Err((status, msg)) => {
                let mut err = ErrorResponse::new(Some(msg.to_string()));
                *err.status_mut() = status;
                Err(err)
            }
        }
    }).await {
        Ok(ws) => ws,
        Err(e) => {
            // Ralentit les tentatives de jeton en série.
            tokio::time::sleep(Duration::from_millis(500)).await;
            return Err(anyhow::anyhow!("handshake refusé : {e}"));
        }
    };
    log::info!("WS: client authentifié");
    let (mut sink, mut source) = ws.split();

    // Statut initial immédiat
    sink.send(Message::Text(to_json(WsEvent::Status(get_status(&app))))).await?;

    // Canal interne : push_task + cmd_task → write loop
    let (tx, mut rx) = tokio::sync::mpsc::channel::<String>(32);

    // Push statut toutes les secondes
    let tx_push = tx.clone();
    let app_push = app.clone();
    let push_task = tokio::spawn(async move {
        let mut tick = tokio::time::interval(Duration::from_secs(1));
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
                    let resp = match serde_json::from_str::<WsCmd>(&text) {
                        Ok(WsCmd { cmd }) => dispatch(&app_cmd, &cmd).await,
                        Err(_) => to_json(WsEvent::Error { message: "Message invalide".into() }),
                    };
                    if tx_cmd.send(resp).await.is_err() {
                        break;
                    }
                }
                Ok(Message::Close(_)) | Err(_) => break,
                _ => {} // Ping/Pong gérés par tungstenite
            }
        }
    });

    // Boucle d'écriture : vide le canal → sink WebSocket
    drop(tx);
    while let Some(msg) = rx.recv().await {
        if sink.send(Message::Text(msg)).await.is_err() {
            break;
        }
        if cmd_task.is_finished() {
            break;
        }
    }

    push_task.abort();
    cmd_task.abort();
    Ok(())
}

// ── Dispatch des commandes ───────────────────────────────────────────────────

async fn dispatch(app: &AppHandle, cmd: &str) -> String {
    let status_or_error = |r: crate::error::YawrecResult<()>| match r {
        Ok(()) => to_json(WsEvent::Status(get_status(app))),
        Err(e) => to_json(WsEvent::Error { message: e.to_string() }),
    };

    match cmd {
        "start" => status_or_error(crate::commands::do_start_recording(app).await),

        "stop" => match crate::commands::do_stop_recording(app).await {
            Ok(path) => to_json(WsEvent::Stopped { path }),
            Err(e)   => to_json(WsEvent::Error { message: e.to_string() }),
        },

        "pause"  => status_or_error(crate::commands::do_pause_recording(app)),
        "resume" => status_or_error(crate::commands::do_resume_recording(app)),

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

#[cfg(test)]
mod tests {
    use super::*;

    fn req(uri: &str, origin: Option<&str>) -> Request {
        let mut b = Request::builder().uri(uri);
        if let Some(o) = origin {
            b = b.header("Origin", o);
        }
        b.body(()).unwrap()
    }

    #[test]
    fn jeton_genere_valide_et_variable() {
        let a = generate_token();
        let b = generate_token();
        assert!(is_valid_token(&a), "{a}");
        assert!(is_valid_token(&b), "{b}");
        assert_ne!(a, b);
    }

    #[test]
    fn handshake_exige_le_jeton() {
        let tok = "ABCDEFGH23";
        assert!(check_handshake(&req("/?token=ABCDEFGH23", None), tok).is_ok());
        assert!(check_handshake(&req("/?x=1&token=ABCDEFGH23", None), tok).is_ok());
        assert_eq!(check_handshake(&req("/", None), tok).unwrap_err().0, StatusCode::UNAUTHORIZED);
        assert_eq!(check_handshake(&req("/?token=ABCDEFGH24", None), tok).unwrap_err().0, StatusCode::UNAUTHORIZED);
    }

    #[test]
    fn handshake_refuse_les_navigateurs() {
        let tok = "ABCDEFGH23";
        let r = check_handshake(&req("/?token=ABCDEFGH23", Some("https://evil.example")), tok);
        assert_eq!(r.unwrap_err().0, StatusCode::FORBIDDEN);
    }
}
