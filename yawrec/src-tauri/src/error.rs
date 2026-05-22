// ============================================================
// YAWREC · error.rs
// Type d'erreur unique remonté à l'UI via les commands.
// Tauri sérialise les erreurs en JSON, on implémente Serialize.
// ============================================================

use serde::Serialize;

#[derive(Debug, thiserror::Error)]
pub enum YawrecError {
    #[error("Capture : {0}")]
    Capture(String),

    #[error("Encodage : {0}")]
    Encoding(String),

    #[error("Périphérique : {0}")]
    Device(String),

    #[error("État invalide : {0}")]
    InvalidState(String),

    #[error("Configuration : {0}")]
    Config(String),

    #[error("E/S : {0}")]
    Io(#[from] std::io::Error),
}

// Tauri exige que les erreurs renvoyées par les commands soient Serialize.
// On sérialise simplement en string : suffisant côté UI.
impl Serialize for YawrecError {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(&self.to_string())
    }
}

pub type YawrecResult<T> = Result<T, YawrecError>;
