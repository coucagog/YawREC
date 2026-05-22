// ============================================================
// YAWREC · capture/audio.rs (D3)
// Capture audio cross-OS via `cpal`.
//
// Architecture :
//   - AudioCapturer doit être créé et détruit sur le MÊME thread
//     (cpal::Stream est !Send sur la plupart des plateformes).
//   - Deux sources potentielles : micro (default input) et loopback
//     système (Windows-only via cpal WASAPI : on ouvre le default
//     output device en mode input → c'est du loopback).
//   - Chaque cpal callback convertit à la volée en f32 stéréo @48kHz
//     puis pousse dans un VecDeque partagé (par source).
//   - pull_chunk() draine 10 ms (480 frames * 2 canaux) depuis chaque
//     source, mixe par somme avec écrêtage [-1, 1], et retourne le buffer.
//
// Le worker audio externe (commands.rs) appelle pull_chunk() en boucle
// puis pousse le résultat dans l'encoder partagé.
// ============================================================

use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use cpal::traits::{DeviceTrait, HostTrait, StreamTrait};

use crate::commands::DeviceInfo;

#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    #[error("Énumération : {0}")]
    Enumeration(String),
    #[error("Périphérique non trouvé : {0}")]
    NotFound(String),
    #[error("Format non supporté : {0}")]
    Format(String),
    #[error("Stream : {0}")]
    Stream(String),
}

pub const TARGET_SAMPLE_RATE: u32 = 48_000;
pub const TARGET_CHANNELS:    u16 = 2;
/// 10 ms à 48 kHz stéréo = 480 frames * 2 canaux = 960 f32.
pub const CHUNK_FRAMES: usize = 480;
pub const CHUNK_SAMPLES: usize = CHUNK_FRAMES * TARGET_CHANNELS as usize;
/// Plafond du buffer par source pour éviter la croissance non bornée
/// si le consommateur est plus lent que les callbacks cpal.
const MAX_BUFFER_SAMPLES: usize = TARGET_SAMPLE_RATE as usize * TARGET_CHANNELS as usize * 2; // 2 secondes

// ============================================================
// Énumération (commande IPC)
// ============================================================

pub fn list_devices() -> Result<Vec<DeviceInfo>, AudioError> {
    let host = cpal::default_host();
    let mut out = Vec::new();

    if let Ok(inputs) = host.input_devices() {
        for d in inputs {
            let name = d.name().unwrap_or_else(|_| "Inconnu".into());
            out.push(DeviceInfo {
                id: format!("input::{name}"),
                name: format!("🎤 {name}"),
            });
        }
    }
    if let Ok(outputs) = host.output_devices() {
        for d in outputs {
            let name = d.name().unwrap_or_else(|_| "Inconnu".into());
            out.push(DeviceInfo {
                id: format!("loopback::{name}"),
                name: format!("🔊 {name} (système)"),
            });
        }
    }
    log::debug!("audio::list_devices → {} entrées", out.len());
    Ok(out)
}

// ============================================================
// AudioCapturer
// ============================================================

pub struct AudioCapturer {
    // Les streams doivent rester vivants pour que les callbacks tournent.
    _streams: Vec<cpal::Stream>,
    mic_buffer:      Arc<Mutex<VecDeque<f32>>>,
    loopback_buffer: Arc<Mutex<VecDeque<f32>>>,
    has_mic: bool,
    has_loopback: bool,
}

impl AudioCapturer {
    /// Ouvre les streams selon la configuration utilisateur. À appeler sur
    /// le thread qui possédera le capturer.
    ///
    /// - `mic_enabled` : ouvre le micro (device par nom si `mic_device_name` fourni, sinon défaut)
    /// - `loopback_enabled` : ouvre le loopback système (Windows uniquement)
    /// - `mic_device_name` : nom exact du device micro (tel que renvoyé par `list_devices`)
    pub fn open(
        mic_enabled: bool,
        loopback_enabled: bool,
        mic_device_name: Option<&str>,
    ) -> Result<Self, AudioError> {
        let host = cpal::default_host();

        let mic_buffer:      Arc<Mutex<VecDeque<f32>>> = Arc::new(Mutex::new(VecDeque::new()));
        let loopback_buffer: Arc<Mutex<VecDeque<f32>>> = Arc::new(Mutex::new(VecDeque::new()));
        let mut streams = Vec::with_capacity(2);
        let mut has_mic = false;
        let mut has_loopback = false;

        // ---- Micro ----
        if mic_enabled {
            let mic_dev = if let Some(name) = mic_device_name {
                // Cherche le device par nom, fallback sur le défaut
                host.input_devices()
                    .ok()
                    .and_then(|mut devs| devs.find(|d| d.name().ok().as_deref() == Some(name)))
                    .or_else(|| host.default_input_device())
            } else {
                host.default_input_device()
            };

            match mic_dev {
                Some(dev) => {
                    let name = dev.name().unwrap_or_else(|_| "?".into());
                    match build_input_stream(&dev, Arc::clone(&mic_buffer), &format!("mic[{name}]")) {
                        Ok(stream) => {
                            if let Err(e) = stream.play() {
                                log::warn!("mic.play() : {e}");
                            } else {
                                log::info!("Audio · micro ouvert : {name}");
                                streams.push(stream);
                                has_mic = true;
                            }
                        }
                        Err(e) => log::warn!("Mic indisponible ({name}) : {e}"),
                    }
                }
                None => log::warn!("Aucun micro disponible"),
            }
        } else {
            log::info!("Audio · micro désactivé par l'utilisateur");
        }

        // ---- Loopback système (Windows uniquement via cpal WASAPI) ----
        #[cfg(target_os = "windows")]
        if loopback_enabled {
            match host.default_output_device() {
                Some(dev) => {
                    let name = dev.name().unwrap_or_else(|_| "?".into());
                    match build_loopback_stream(&dev, Arc::clone(&loopback_buffer), &format!("loop[{name}]")) {
                        Ok(stream) => {
                            if let Err(e) = stream.play() {
                                log::warn!("loopback.play() : {e}");
                            } else {
                                log::info!("Audio · loopback système ouvert : {name}");
                                streams.push(stream);
                                has_loopback = true;
                            }
                        }
                        Err(e) => log::warn!("Loopback indisponible ({name}) : {e}"),
                    }
                }
                None => log::warn!("Aucune sortie audio par défaut (pas de loopback)"),
            }
        } else {
            log::info!("Audio · loopback désactivé par l'utilisateur");
        }

        #[cfg(not(target_os = "windows"))]
        log::info!("Audio · loopback système non implémenté sur cette plateforme");

        if !has_mic && !has_loopback {
            return Err(AudioError::NotFound(
                "Aucune source audio disponible (ni micro, ni loopback)".into(),
            ));
        }

        Ok(Self {
            _streams: streams,
            mic_buffer,
            loopback_buffer,
            has_mic,
            has_loopback,
        })
    }

    pub fn has_mic(&self) -> bool { self.has_mic }
    pub fn has_loopback(&self) -> bool { self.has_loopback }

    /// Draine 10 ms depuis chaque source disponible et les mixe.
    ///
    /// Stratégie de synchronisation :
    /// - Si UNE SEULE source est active, on draine dès que CHUNK_SAMPLES sont disponibles.
    /// - Si LES DEUX sources sont actives, on attend que LES DEUX aient CHUNK_SAMPLES avant
    ///   de drainer l'une ou l'autre. Cela évite le problème de chunks alternant entre
    ///   mic-only et loopback-only, qui produit du bruit par désynchronisation.
    ///
    /// Retourne `None` si pas encore assez de données — l'appelant réessaiera dans 10 ms.
    pub fn pull_chunk(&self) -> Option<Vec<f32>> {
        // Vérifier AVANT de drainer que chaque source active a assez de données.
        if self.has_mic {
            let ready = self.mic_buffer.lock().ok().map_or(0, |g| g.len()) >= CHUNK_SAMPLES;
            if !ready { return None; }
        }
        if self.has_loopback {
            let ready = self.loopback_buffer.lock().ok().map_or(0, |g| g.len()) >= CHUNK_SAMPLES;
            if !ready { return None; }
        }

        let mic_chunk  = if self.has_mic      { drain_chunk(&self.mic_buffer)      } else { None };
        let loop_chunk = if self.has_loopback  { drain_chunk(&self.loopback_buffer) } else { None };

        if mic_chunk.is_none() && loop_chunk.is_none() {
            return None;
        }

        let mut mixed = vec![0.0f32; CHUNK_SAMPLES];
        if let Some(ref m) = mic_chunk {
            for i in 0..CHUNK_SAMPLES { mixed[i] += m[i]; }
        }
        if let Some(ref l) = loop_chunk {
            for i in 0..CHUNK_SAMPLES { mixed[i] += l[i]; }
        }

        // Normalisation du mix : diviser par 2 si deux sources, clamp de sécurité.
        if mic_chunk.is_some() && loop_chunk.is_some() {
            for s in mixed.iter_mut() { *s *= 0.5; }
        }
        for s in mixed.iter_mut() { *s = s.clamp(-1.0, 1.0); }

        Some(mixed)
    }
}

/// Sort exactement CHUNK_SAMPLES depuis le buffer, ou None si pas assez.
fn drain_chunk(buf: &Arc<Mutex<VecDeque<f32>>>) -> Option<Vec<f32>> {
    let mut g = buf.lock().ok()?;
    if g.len() < CHUNK_SAMPLES {
        return None;
    }
    let chunk: Vec<f32> = g.drain(..CHUNK_SAMPLES).collect();
    Some(chunk)
}

// ============================================================
// Construction de stream cpal (mic ou loopback)
// ============================================================

/// Loopback stream from an output device (WASAPI loopback on Windows).
/// Must use the device's *output* config, not input config.
#[cfg(target_os = "windows")]
fn build_loopback_stream(
    device: &cpal::Device,
    target: Arc<Mutex<VecDeque<f32>>>,
    label: &str,
) -> Result<cpal::Stream, AudioError> {
    let supported_config = device
        .default_output_config()
        .map_err(|e| AudioError::Format(format!("{label} default_output_config : {e}")))?;
    build_stream_inner(device, supported_config, target, label)
}

fn build_input_stream(
    device: &cpal::Device,
    target: Arc<Mutex<VecDeque<f32>>>,
    label: &str,
) -> Result<cpal::Stream, AudioError> {
    let supported_config = device
        .default_input_config()
        .map_err(|e| AudioError::Format(format!("{label} default_input_config : {e}")))?;
    build_stream_inner(device, supported_config, target, label)
}

fn build_stream_inner(
    device: &cpal::Device,
    supported_config: cpal::SupportedStreamConfig,
    target: Arc<Mutex<VecDeque<f32>>>,
    label: &str,
) -> Result<cpal::Stream, AudioError> {

    let sample_format = supported_config.sample_format();
    let stream_config: cpal::StreamConfig = supported_config.into();
    let sample_rate = stream_config.sample_rate.0;
    let channels    = stream_config.channels;

    log::info!(
        "{label} : {} Hz, {} canaux, format {:?}",
        sample_rate, channels, sample_format,
    );

    let label_owned = label.to_string();
    let err_label   = label.to_string();
    let err_fn = move |err| log::error!("{err_label} stream : {err}");

    macro_rules! build {
        ($ty:ty) => {{
            let target = Arc::clone(&target);
            let label_cb = label_owned.clone();
            device.build_input_stream(
                &stream_config,
                move |data: &[$ty], _: &cpal::InputCallbackInfo| {
                    // 1. Sample → f32 in [-1, 1]
                    let f32_samples: Vec<f32> = data.iter().map(|s| sample_to_f32(*s)).collect();
                    // 2. Convert (channels, sample_rate) → (2, 48kHz)
                    let converted = convert_to_48k_stereo(&f32_samples, sample_rate, channels);
                    // 3. Push, avec plafond
                    if let Ok(mut g) = target.lock() {
                        g.extend(converted);
                        if g.len() > MAX_BUFFER_SAMPLES {
                            let drop = g.len() - MAX_BUFFER_SAMPLES;
                            log::trace!("{label_cb} backpressure : drop {drop} samples");
                            g.drain(..drop);
                        }
                    }
                },
                err_fn.clone(),
                Some(Duration::from_millis(50)),
            )
        }};
    }

    let stream = match sample_format {
        cpal::SampleFormat::F32 => build!(f32),
        cpal::SampleFormat::I16 => build!(i16),
        cpal::SampleFormat::U16 => build!(u16),
        cpal::SampleFormat::I32 => build!(i32),
        cpal::SampleFormat::I8  => build!(i8),
        cpal::SampleFormat::U8  => build!(u8),
        other => return Err(AudioError::Format(
            format!("{label} format non supporté : {other:?}")
        )),
    }
    .map_err(|e| AudioError::Stream(format!("{label} build_input_stream : {e}")))?;

    Ok(stream)
}

// ============================================================
// Conversion d'échantillons
// ============================================================

trait SampleToF32 { fn to_f32(self) -> f32; }
impl SampleToF32 for f32 { fn to_f32(self) -> f32 { self } }
impl SampleToF32 for i16 { fn to_f32(self) -> f32 { self as f32 / i16::MAX as f32 } }
impl SampleToF32 for u16 { fn to_f32(self) -> f32 { (self as f32 - 32768.0) / 32768.0 } }
impl SampleToF32 for i32 { fn to_f32(self) -> f32 { self as f32 / i32::MAX as f32 } }
impl SampleToF32 for i8  { fn to_f32(self) -> f32 { self as f32 / i8::MAX as f32 } }
impl SampleToF32 for u8  { fn to_f32(self) -> f32 { (self as f32 - 128.0) / 128.0 } }

fn sample_to_f32<S: SampleToF32 + Copy>(s: S) -> f32 { s.to_f32() }

/// Convertit (channels, sample_rate) → (2, 48000).
///
/// 1. Channel mapping :
///    - mono → stéréo : duplication L=R=M
///    - stéréo → stéréo : pass-through
///    - multi → stéréo : on garde les 2 premiers canaux
///
/// 2. Resampling : interpolation linéaire. Suffisant pour de l'audio
///    de capture (parole + jeu) ; pour de la musique pure on voudrait
///    un sinc resampler (rubato), prochain pas si besoin.
fn convert_to_48k_stereo(input: &[f32], sample_rate: u32, channels: u16) -> Vec<f32> {
    if input.is_empty() { return Vec::new(); }

    // ---- Étape 1 : channel mapping → stéréo ----
    let stereo: Vec<f32> = match channels {
        1 => input.iter().flat_map(|&s| [s, s]).collect(),
        // Fold L+R → mono symétrique : règle les devices stéréo qui ne remplissent
        // qu'un seul canal (ex. webcam mic C270 via WASAPI qui envoie [signal, 0]).
        2 => input.chunks_exact(2)
            .flat_map(|pair| {
                let m = (pair[0] + pair[1]) * 0.5;
                [m, m]
            })
            .collect(),
        n => {
            let n = n as usize;
            let mut out = Vec::with_capacity((input.len() / n) * 2);
            for chunk in input.chunks_exact(n) {
                let m = (chunk[0] + chunk[1]) * 0.5;
                out.push(m);
                out.push(m);
            }
            out
        }
    };

    // ---- Étape 2 : resample ----
    if sample_rate == TARGET_SAMPLE_RATE {
        return stereo;
    }

    let ratio = TARGET_SAMPLE_RATE as f64 / sample_rate as f64;
    let in_frames  = stereo.len() / 2;
    let out_frames = (in_frames as f64 * ratio).round() as usize;
    let mut out = Vec::with_capacity(out_frames * 2);

    for i in 0..out_frames {
        let src_pos = i as f64 / ratio;
        let src_idx = src_pos as usize;
        let frac    = (src_pos - src_idx as f64) as f32;

        if src_idx + 1 < in_frames {
            let l1 = stereo[src_idx * 2];
            let r1 = stereo[src_idx * 2 + 1];
            let l2 = stereo[(src_idx + 1) * 2];
            let r2 = stereo[(src_idx + 1) * 2 + 1];
            out.push(l1 + (l2 - l1) * frac);
            out.push(r1 + (r2 - r1) * frac);
        } else if src_idx < in_frames {
            out.push(stereo[src_idx * 2]);
            out.push(stereo[src_idx * 2 + 1]);
        }
    }
    out
}
