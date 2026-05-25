// ============================================================
// YAWREC · capture/mod.rs
// Module racine de capture. Traits abstraits + sous-modules
// par source (écran, audio, webcam).
//
// Convention : chaque sous-module expose
//   - `list_devices()` ou `list_screens()` (énumération)
//   - une struct Capturer avec start() / stop() / next_frame()
// L'implémentation peut être OS-spécifique derrière un cfg(target_os).
// ============================================================

pub mod audio;
pub mod screen;
pub mod webcam;
pub mod window;

use std::time::Duration;

/// Une frame brute issue d'une source de capture.
/// Le format dépend de la source ; pour l'écran et la webcam on convertit
/// systématiquement en BGRA8 avant compositing/encodage.
#[derive(Debug, Clone)]
pub struct Frame {
    pub width: u32,
    pub height: u32,
    pub stride: u32,
    pub data: Vec<u8>,
    /// PTS relatif au début de l'enregistrement.
    pub timestamp: Duration,
}

/// Trait commun à toute source de capture qu'on peut démarrer/arrêter.
pub trait Capturer: Send {
    type Error: std::error::Error + Send + Sync + 'static;

    fn start(&mut self) -> Result<(), Self::Error>;
    fn stop(&mut self) -> Result<(), Self::Error>;
    /// Bloquant : récupère la prochaine frame disponible.
    /// Retourne `None` si la capture est arrêtée.
    fn next_frame(&mut self) -> Result<Option<Frame>, Self::Error>;
}
