// ============================================================
// YAWREC · capture/audio.rs (D3)
// Capture audio cross-OS via `cpal`.
//
// Architecture :
//   - AudioCapturer doit être créé et détruit sur le MÊME thread
//     (cpal::Stream est !Send sur la plupart des plateformes).
//   - Deux sources potentielles : micro et loopback système (Windows :
//     le default output device ouvert en input = loopback WASAPI).
//   - Chaque callback cpal convertit en f32 stéréo @48 kHz (rééchantillonneur
//     continu d'un callback à l'autre) puis écrit dans un `SourceBuffer`
//     indexé en frames depuis l'epoch de l'enregistrement.
//   - pull_chunk() émet 10 ms d'audio à la fois, cadencé sur l'horloge murale
//     avec un retard fixe (OUTPUT_DELAY). Une source muette ou en retard est
//     complétée par du silence : le loopback WASAPI n'envoie RIEN tant
//     qu'aucun son ne joue, et sans cette horloge la piste audio se
//     "contractait" (désync) ou bloquait le micro (trous).
//
// Le worker audio externe (commands.rs) appelle pull_chunk() en boucle
// puis pousse le résultat dans l'encoder partagé.
// ============================================================

use std::collections::VecDeque;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};
use std::sync::{Arc, Mutex};
use std::thread::JoinHandle;
use std::time::{Duration, Instant};

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

const fn ms_to_frames(ms: u64) -> u64 { TARGET_SAMPLE_RATE as u64 * ms / 1000 }

/// Retard d'émission : on émet l'audio de (maintenant − OUTPUT_DELAY), ce qui
/// laisse aux callbacks le temps d'arriver. Il n'affecte pas la synchro A/V :
/// chaque chunk porte sa position dans le temps.
const OUTPUT_DELAY_FRAMES: u64 = ms_to_frames(250);
/// Au-delà de cet écart entre la fin des données d'une source et l'instant
/// du callback, on considère qu'il y a eu un trou (silence loopback).
const GAP_TOLERANCE_FRAMES: u64 = ms_to_frames(100);
/// Avance maximale tolérée d'une source sur l'horloge murale (device dont
/// l'horloge est un peu rapide) ; au-delà les frames sont jetées.
const MAX_AHEAD_FRAMES: u64 = ms_to_frames(100);
/// Plafond du buffer par source (2 s) pour éviter une croissance non bornée.
const MAX_BUFFER_SAMPLES: usize = TARGET_SAMPLE_RATE as usize * TARGET_CHANNELS as usize * 2;

fn wall_frames(epoch: Instant) -> u64 {
    (epoch.elapsed().as_nanos() * TARGET_SAMPLE_RATE as u128 / 1_000_000_000) as u64
}

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
// SourceBuffer — échantillons d'une source positionnés dans le temps
// ============================================================

/// Échantillons stéréo d'une source, avec la position (en frames depuis
/// l'epoch) du premier échantillon. Le producteur (callback cpal) date les
/// données à l'arrivée ; le consommateur lit une fenêtre [pos, pos+n) et
/// complète par du silence ce qui manque.
#[derive(Debug, Default)]
struct SourceBuffer {
    data: VecDeque<f32>,
    head_pos: u64,
    started: bool,
}

impl SourceBuffer {
    fn write_pos(&self) -> u64 {
        self.head_pos + (self.data.len() / 2) as u64
    }

    /// `samples` (stéréo) dont la dernière frame vient d'être capturée à
    /// l'instant `now` (frames depuis l'epoch).
    fn push(&mut self, samples: &[f32], now: u64) {
        let n = (samples.len() / 2) as u64;
        if n == 0 {
            return;
        }
        let start = now.saturating_sub(n);

        if !self.started {
            self.started = true;
            self.head_pos = start;
        } else if start > self.write_pos() + GAP_TOLERANCE_FRAMES {
            // Trou (source muette) : recaler la source sur l'horloge.
            if self.data.is_empty() {
                self.head_pos = start;
            } else {
                let gap = (start - self.write_pos()) as usize;
                self.data.extend(std::iter::repeat(0.0).take(gap * 2));
            }
        }

        // Source en avance sur l'horloge murale : jeter l'excédent.
        let mut skip = 0usize;
        if self.write_pos() > now + MAX_AHEAD_FRAMES {
            skip = ((self.write_pos() - now) as usize).min(n as usize);
        }
        self.data.extend(&samples[skip * 2..]);

        if self.data.len() > MAX_BUFFER_SAMPLES {
            let excess = (self.data.len() - MAX_BUFFER_SAMPLES) / 2;
            self.data.drain(..excess * 2);
            self.head_pos += excess as u64;
        }
    }

    /// Remplit `out` (stéréo) avec les frames [pos, pos + out.len()/2).
    /// Les frames absentes sont du silence. Retourne true si au moins une
    /// frame réelle a été lue.
    fn read_into(&mut self, pos: u64, out: &mut [f32]) -> bool {
        out.fill(0.0);
        let frames = (out.len() / 2) as u64;

        // Données trop vieilles (arrivées après leur fenêtre) : jetées.
        if self.head_pos < pos {
            let drop = (pos - self.head_pos).min((self.data.len() / 2) as u64);
            self.data.drain(..drop as usize * 2);
            self.head_pos += drop;
        }
        if self.data.is_empty() || self.head_pos < pos || self.head_pos >= pos + frames {
            return false;
        }

        let offset = (self.head_pos - pos) as usize;
        let take = ((frames as usize) - offset).min(self.data.len() / 2);
        for (dst, src) in out[offset * 2..(offset + take) * 2].iter_mut().zip(self.data.drain(..take * 2)) {
            *dst = src;
        }
        self.head_pos += take as u64;
        take > 0
    }
}

// ============================================================
// AudioCapturer
// ============================================================

pub struct AudioCapturer {
    // Les streams doivent rester vivants pour que les callbacks tournent.
    _streams: Vec<cpal::Stream>,
    mic:      Option<Arc<Mutex<SourceBuffer>>>,
    loopback: Option<Arc<Mutex<SourceBuffer>>>,
    epoch: Instant,
    /// Position (frames depuis l'epoch) du prochain chunk à émettre.
    emit_pos: u64,
    mic_gain:  Arc<AtomicU32>,
    mic_level: Arc<AtomicU32>, // VU partagé avec RecorderState
}

impl AudioCapturer {
    /// Ouvre les streams selon la configuration utilisateur. À appeler sur
    /// le thread qui possédera le capturer.
    ///
    /// - `mic_enabled` : ouvre le micro (device par nom si `mic_device_name` fourni, sinon défaut)
    /// - `loopback_enabled` : ouvre le loopback système (Windows uniquement)
    /// - `epoch` : origine temporelle de l'enregistrement (partagée avec la vidéo)
    pub fn open(
        mic_enabled: bool,
        loopback_enabled: bool,
        mic_device_name: Option<&str>,
        mic_gain:  Arc<AtomicU32>,
        mic_level: Arc<AtomicU32>,
        epoch: Instant,
    ) -> Result<Self, AudioError> {
        let host = cpal::default_host();
        let mut streams = Vec::with_capacity(2);
        let mut mic = None;
        let mut loopback = None;

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
                    let buf = Arc::new(Mutex::new(SourceBuffer::default()));
                    match build_input_stream(&dev, Arc::clone(&buf), epoch, &format!("mic[{name}]")) {
                        Ok(stream) => {
                            if let Err(e) = stream.play() {
                                log::warn!("mic.play() : {e}");
                            } else {
                                log::info!("Audio · micro ouvert : {name}");
                                streams.push(stream);
                                mic = Some(buf);
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
                    let buf = Arc::new(Mutex::new(SourceBuffer::default()));
                    match build_loopback_stream(&dev, Arc::clone(&buf), epoch, &format!("loop[{name}]")) {
                        Ok(stream) => {
                            if let Err(e) = stream.play() {
                                log::warn!("loopback.play() : {e}");
                            } else {
                                log::info!("Audio · loopback système ouvert : {name}");
                                streams.push(stream);
                                loopback = Some(buf);
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
        if loopback_enabled {
            log::info!("Audio · loopback système non implémenté sur cette plateforme");
        }

        if mic.is_none() && loopback.is_none() {
            return Err(AudioError::NotFound(
                "Aucune source audio disponible (ni micro, ni loopback)".into(),
            ));
        }

        Ok(Self {
            _streams: streams,
            mic,
            loopback,
            epoch,
            emit_pos: 0,
            mic_gain,
            mic_level,
        })
    }

    pub fn has_mic(&self) -> bool { self.mic.is_some() }
    pub fn has_loopback(&self) -> bool { self.loopback.is_some() }

    /// Émet le prochain chunk de 10 ms s'il est "dû" sur l'horloge murale.
    /// Retourne (samples stéréo, position en ms depuis l'epoch) ; None si
    /// c'est encore trop tôt — l'appelant réessaiera.
    pub fn pull_chunk(&mut self) -> Option<(Vec<f32>, u64)> {
        let now = wall_frames(self.epoch);
        if now < self.emit_pos + CHUNK_FRAMES as u64 + OUTPUT_DELAY_FRAMES {
            return None;
        }
        let pos = self.emit_pos;
        self.emit_pos += CHUNK_FRAMES as u64;

        let mut mixed = vec![0.0f32; CHUNK_SAMPLES];

        if let Some(mic) = &self.mic {
            let mut m = vec![0.0f32; CHUNK_SAMPLES];
            if let Ok(mut g) = mic.lock() {
                g.read_into(pos, &mut m);
            }
            // Gain micro appliqué avant mix (modifiable en live via AtomicU32).
            let gain = f32::from_bits(self.mic_gain.load(Ordering::Relaxed));
            if (gain - 1.0).abs() > f32::EPSILON {
                for s in m.iter_mut() { *s = (*s * gain).clamp(-1.0, 1.0); }
            }
            // VU-mètre : RMS après gain, attaque immédiate / déclin lent.
            let rms = (m.iter().map(|&s| s * s).sum::<f32>() / m.len() as f32).sqrt();
            let prev = f32::from_bits(self.mic_level.load(Ordering::Relaxed));
            let next = if rms >= prev { rms } else { (prev * 0.96).max(0.0) };
            self.mic_level.store(next.to_bits(), Ordering::Relaxed);

            for (d, s) in mixed.iter_mut().zip(m) { *d += s; }
        }

        if let Some(lb) = &self.loopback {
            let mut l = vec![0.0f32; CHUNK_SAMPLES];
            if let Ok(mut g) = lb.lock() {
                g.read_into(pos, &mut l);
            }
            for (d, s) in mixed.iter_mut().zip(l) { *d += s; }
        }

        // Mix par somme (comme OBS) avec écrêtage de sécurité : diviser par 2
        // faisait perdre 6 dB aux deux sources dès que l'une était silencieuse.
        for s in mixed.iter_mut() { *s = s.clamp(-1.0, 1.0); }

        Some((mixed, pos * 1000 / TARGET_SAMPLE_RATE as u64))
    }
}

// ============================================================
// Construction de stream cpal (mic ou loopback)
// ============================================================

/// Loopback stream from an output device (WASAPI loopback on Windows).
/// Must use the device's *output* config, not input config.
#[cfg(target_os = "windows")]
fn build_loopback_stream(
    device: &cpal::Device,
    target: Arc<Mutex<SourceBuffer>>,
    epoch: Instant,
    label: &str,
) -> Result<cpal::Stream, AudioError> {
    let supported_config = device
        .default_output_config()
        .map_err(|e| AudioError::Format(format!("{label} default_output_config : {e}")))?;
    // Son système : on garde la stéréo (et on downmixe le multicanal).
    build_stream_inner(device, supported_config, target, epoch, ChannelMode::Stereo, label)
}

fn build_input_stream(
    device: &cpal::Device,
    target: Arc<Mutex<SourceBuffer>>,
    epoch: Instant,
    label: &str,
) -> Result<cpal::Stream, AudioError> {
    let supported_config = device
        .default_input_config()
        .map_err(|e| AudioError::Format(format!("{label} default_input_config : {e}")))?;
    // Micro : repli mono (gère les devices stéréo qui ne remplissent qu'un
    // canal, ex. webcam C270 via WASAPI qui envoie [signal, 0]).
    build_stream_inner(device, supported_config, target, epoch, ChannelMode::MonoFold, label)
}

fn build_stream_inner(
    device: &cpal::Device,
    supported_config: cpal::SupportedStreamConfig,
    target: Arc<Mutex<SourceBuffer>>,
    epoch: Instant,
    mode: ChannelMode,
    label: &str,
) -> Result<cpal::Stream, AudioError> {

    let sample_format = supported_config.sample_format();
    let stream_config: cpal::StreamConfig = supported_config.into();
    let sample_rate = stream_config.sample_rate.0;
    let channels    = stream_config.channels;

    log::info!(
        "{label} : {} Hz, {} canaux, format {:?}, mode {:?}",
        sample_rate, channels, sample_format, mode,
    );

    let err_label   = label.to_string();
    let err_fn = move |err| log::error!("{err_label} stream : {err}");

    macro_rules! build {
        ($ty:ty) => {{
            let target = Arc::clone(&target);
            let mut resampler = Resampler::new(sample_rate);
            device.build_input_stream(
                &stream_config,
                move |data: &[$ty], _: &cpal::InputCallbackInfo| {
                    let now = wall_frames(epoch);
                    // 1. Sample → f32 in [-1, 1]
                    let f32_samples: Vec<f32> = data.iter().map(|s| sample_to_f32(*s)).collect();
                    // 2. (channels, sample_rate) → (2, 48 kHz)
                    let stereo = map_channels(&f32_samples, channels, mode);
                    let converted = resampler.process(&stereo);
                    // 3. Push daté
                    if let Ok(mut g) = target.lock() {
                        g.push(&converted, now);
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ChannelMode {
    /// Micro : un seul signal utile, dupliqué sur L et R.
    MonoFold,
    /// Son système : stéréo conservée, multicanal downmixé.
    Stereo,
}

/// Convertit un buffer interleavé `channels` canaux en stéréo interleavé.
fn map_channels(input: &[f32], channels: u16, mode: ChannelMode) -> Vec<f32> {
    let n = channels.max(1) as usize;
    let frames = input.chunks_exact(n);
    if !frames.remainder().is_empty() {
        log::warn!("map_channels : {} sample(s) incomplets droppés ({n}-ch)", frames.remainder().len());
    }

    match (n, mode) {
        (1, _) => input.iter().flat_map(|&s| [s, s]).collect(),
        (_, ChannelMode::MonoFold) => {
            // Si un des deux premiers canaux est quasi vide sur ce buffer,
            // on garde l'autre tel quel (sinon la moyenne perdrait 6 dB).
            let (mut el, mut er) = (0.0f32, 0.0f32);
            for f in input.chunks_exact(n) {
                el += f[0] * f[0];
                er += f[1] * f[1];
            }
            let pick: fn(&[f32]) -> f32 = if er < el * 0.01 {
                |f| f[0]
            } else if el < er * 0.01 {
                |f| f[1]
            } else {
                |f| (f[0] + f[1]) * 0.5
            };
            input.chunks_exact(n).flat_map(|f| { let m = pick(f); [m, m] }).collect()
        }
        (2, ChannelMode::Stereo) => input[..input.len() / 2 * 2].to_vec(),
        (_, ChannelMode::Stereo) => {
            // Ordre WAVE : FL FR FC LFE BL BR SL SR. LFE ignoré, le reste
            // replié à -3 dB sur son côté, puis normalisé.
            const K: f32 = std::f32::consts::FRAC_1_SQRT_2;
            let side = |f: &[f32], idx: &[usize]| -> (f32, f32) {
                idx.iter().filter(|&&i| i < f.len()).fold((0.0, 0.0), |(s, w), &i| (s + f[i] * K, w + K))
            };
            input.chunks_exact(n).flat_map(|f| {
                let (cl, wl) = side(f, &[2, 4, 6]);
                let (cr, wr) = side(f, &[2, 5, 7]);
                [(f[0] + cl) / (1.0 + wl), (f[1] + cr) / (1.0 + wr)]
            }).collect()
        }
    }
}

/// Rééchantillonneur linéaire stéréo vers 48 kHz qui conserve son état
/// d'un callback à l'autre (pas de discontinuité ni de dérive d'arrondi
/// aux frontières de buffer, contrairement à un traitement par buffer).
struct Resampler {
    /// Frames d'entrée par frame de sortie.
    step: f64,
    /// Position de la prochaine frame de sortie, en frames d'entrée, relative
    /// au début du buffer courant (−1 ≤ t < 0 désigne la frame `prev`).
    t: f64,
    prev: Option<[f32; 2]>,
}

impl Resampler {
    fn new(in_rate: u32) -> Self {
        Self { step: in_rate as f64 / TARGET_SAMPLE_RATE as f64, t: 0.0, prev: None }
    }

    fn process(&mut self, input: &[f32]) -> Vec<f32> {
        if (self.step - 1.0).abs() < f64::EPSILON {
            return input.to_vec();
        }
        let n = input.len() / 2;
        if n == 0 {
            return Vec::new();
        }
        let frame = |i: i64, prev: Option<[f32; 2]>| -> [f32; 2] {
            if i < 0 {
                prev.unwrap_or([input[0], input[1]])
            } else {
                [input[i as usize * 2], input[i as usize * 2 + 1]]
            }
        };

        let mut out = Vec::with_capacity(((n as f64 / self.step) as usize + 2) * 2);
        loop {
            let i = self.t.floor() as i64;
            if i + 1 >= n as i64 {
                break;
            }
            let frac = (self.t - i as f64) as f32;
            let a = frame(i, self.prev);
            let b = frame(i + 1, self.prev);
            out.push(a[0] + (b[0] - a[0]) * frac);
            out.push(a[1] + (b[1] - a[1]) * frac);
            self.t += self.step;
        }
        self.t -= n as f64;
        self.prev = Some([input[(n - 1) * 2], input[(n - 1) * 2 + 1]]);
        out
    }
}

// ============================================================
// AudioMonitor — niveau micro toujours actif (pré-enregistrement)
// ============================================================

/// Lance un thread léger qui ouvre le micro, calcule le RMS et
/// l'écrit dans `level` (f32 bits). Tourne jusqu'à ce que `stop` soit true.
///
/// Important : `cpal::Stream` n'est pas `Send` sur WASAPI, donc le stream
/// est créé ET détenu sur le thread monitor lui-même. On utilise un canal
/// sync pour renvoyer succès/erreur à l'appelant.
pub fn start_monitor(
    mic_device_name: Option<&str>,
    level: Arc<AtomicU32>,
    gain:  Arc<AtomicU32>,
    stop:  Arc<AtomicBool>,
) -> Result<JoinHandle<()>, AudioError> {
    let device_name_owned = mic_device_name.map(|s| s.to_string());
    let (tx, rx) = std::sync::mpsc::sync_channel::<Result<(), AudioError>>(1);

    let handle = std::thread::Builder::new()
        .name("yawrec-mic-monitor".into())
        .spawn(move || {
            // Ouverture du device + stream SUR CE thread (Stream n'est pas Send sur WASAPI).
            let host = cpal::default_host();
            let device = if let Some(ref name) = device_name_owned {
                host.input_devices()
                    .ok()
                    .and_then(|mut devs| devs.find(|d| d.name().ok().as_deref() == Some(name.as_str())))
                    .or_else(|| host.default_input_device())
            } else {
                host.default_input_device()
            };

            let device = match device {
                Some(d) => d,
                None => {
                    let _ = tx.send(Err(AudioError::NotFound("Aucun micro (monitor VU)".into())));
                    return;
                }
            };
            let dev_name = device.name().unwrap_or_else(|_| "?".into());

            let supported = match device.default_input_config() {
                Ok(c)  => c,
                Err(e) => { let _ = tx.send(Err(AudioError::Format(format!("monitor cfg : {e}")))); return; }
            };
            let fmt      = supported.sample_format();
            let cfg: cpal::StreamConfig = supported.into();
            let channels = cfg.channels as usize;

            macro_rules! build_mon {
                ($ty:ty) => {{
                    let lv = Arc::clone(&level);
                    let gn = Arc::clone(&gain);
                    device.build_input_stream(
                        &cfg,
                        move |data: &[$ty], _| {
                            if data.is_empty() { return; }
                            let step  = channels.max(1);
                            let count = data.len() / step;
                            if count == 0 { return; }
                            // RMS du canal gauche — reflète le micro principal.
                            let sum_sq: f32 = (0..count)
                                .map(|i| { let s = sample_to_f32(data[i * step]); s * s })
                                .sum();
                            let rms    = (sum_sq / count as f32).sqrt();
                            let g      = f32::from_bits(gn.load(Ordering::Relaxed));
                            let gained = (rms * g).min(1.0);
                            // Ballistics : attaque instantanée, déclin lent.
                            let prev = f32::from_bits(lv.load(Ordering::Relaxed));
                            let next = if gained >= prev { gained } else { (prev * 0.96).max(0.0) };
                            lv.store(next.to_bits(), Ordering::Relaxed);
                        },
                        |e| log::warn!("AudioMonitor stream : {e}"),
                        Some(Duration::from_millis(50)),
                    )
                }};
            }

            let stream_res = match fmt {
                cpal::SampleFormat::F32 => build_mon!(f32),
                cpal::SampleFormat::I16 => build_mon!(i16),
                cpal::SampleFormat::U16 => build_mon!(u16),
                cpal::SampleFormat::I32 => build_mon!(i32),
                cpal::SampleFormat::I8  => build_mon!(i8),
                cpal::SampleFormat::U8  => build_mon!(u8),
                other => {
                    let _ = tx.send(Err(AudioError::Format(format!("monitor format : {other:?}"))));
                    return;
                }
            };

            let stream = match stream_res {
                Ok(s)  => s,
                Err(e) => { let _ = tx.send(Err(AudioError::Stream(format!("monitor build : {e}")))); return; }
            };

            if let Err(e) = stream.play() {
                let _ = tx.send(Err(AudioError::Stream(format!("monitor play : {e}"))));
                return;
            }

            log::info!("AudioMonitor · démarré sur {dev_name}");
            let _ = tx.send(Ok(()));

            // Garde le stream vivant jusqu'à l'arrêt (le stop flag est posé par commands.rs).
            while !stop.load(Ordering::Relaxed) {
                std::thread::sleep(Duration::from_millis(100));
            }
            drop(stream);
            log::info!("AudioMonitor · arrêté");
        })
        .map_err(|e| AudioError::Stream(format!("spawn monitor : {e}")))?;

    // Attendre la confirmation de démarrage (timeout 1 s pour éviter un blocage infini).
    match rx.recv_timeout(Duration::from_secs(1)) {
        Ok(Ok(()))  => Ok(handle),
        Ok(Err(e))  => { let _ = handle.join(); Err(e) }
        Err(_)      => Err(AudioError::Stream("AudioMonitor : timeout au démarrage".into())),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn source_buffer_aligne_sur_la_position() {
        let mut b = SourceBuffer::default();
        // 10 frames capturées, la dernière à t=110 → positions 100..110.
        let samples: Vec<f32> = (0..10).flat_map(|i| [i as f32, i as f32]).collect();
        b.push(&samples, 110);
        let mut out = vec![9.0f32; 20];
        assert!(b.read_into(95, &mut out));
        // 95..100 silence, puis 0..5
        assert_eq!(&out[..10], &[0.0; 10]);
        assert_eq!(out[10], 0.0);
        assert_eq!(out[12], 1.0);
        assert_eq!(out[18], 4.0);
        // la suite reprend à 5
        assert!(b.read_into(105, &mut out));
        assert_eq!(out[0], 5.0);
        assert_eq!(&out[10..], &[0.0; 10]);
    }

    #[test]
    fn source_buffer_comble_les_trous() {
        let mut b = SourceBuffer::default();
        b.push(&[1.0; 20], 10);          // positions 0..10
        // Puis plus rien pendant 1 s (loopback muet), reprise à t=48010.
        b.push(&[2.0; 20], 48_010);      // positions 48000..48010
        let mut out = vec![0.0f32; 20];
        assert!(b.read_into(0, &mut out));
        assert_eq!(out, vec![1.0; 20]);
        b.read_into(24_000, &mut out);
        assert_eq!(out, vec![0.0; 20], "milieu du trou = silence");
        assert!(b.read_into(48_000, &mut out));
        assert_eq!(out, vec![2.0; 20]);
    }

    #[test]
    fn source_buffer_jette_les_donnees_en_retard() {
        let mut b = SourceBuffer::default();
        b.push(&[1.0; 20], 10);
        let mut out = vec![0.0f32; 20];
        assert!(!b.read_into(100, &mut out));
        assert!(b.data.is_empty());
    }

    #[test]
    fn resampler_44k1_produit_le_bon_nombre_de_frames() {
        let mut r = Resampler::new(44_100);
        let mut total = 0usize;
        // 100 buffers de 441 frames = 1 s → ≈ 48000 frames, sans dérive.
        for _ in 0..100 {
            total += r.process(&vec![0.5f32; 882]).len() / 2;
        }
        assert!((total as i64 - 48_000).abs() <= 2, "total={total}");
    }

    #[test]
    fn resampler_continu_aux_frontieres() {
        // Rampe linéaire découpée en buffers : la sortie doit rester une rampe.
        let mut r = Resampler::new(24_000);
        let mut out = Vec::new();
        for k in 0..4 {
            let buf: Vec<f32> = (0..10).flat_map(|i| { let v = (k * 10 + i) as f32; [v, v] }).collect();
            out.extend(r.process(&buf));
        }
        let left: Vec<f32> = out.chunks_exact(2).map(|f| f[0]).collect();
        for w in left.windows(2) {
            assert!((w[1] - w[0] - 0.5).abs() < 1e-4, "saut {:?}", w);
        }
    }

    #[test]
    fn map_channels_modes() {
        // Micro stéréo avec un canal vide → canal actif conservé (pas -6 dB).
        let mono = map_channels(&[0.8, 0.0, 0.6, 0.0], 2, ChannelMode::MonoFold);
        assert_eq!(mono, vec![0.8, 0.8, 0.6, 0.6]);
        // Son système stéréo → inchangé.
        let st = map_channels(&[0.1, 0.9], 2, ChannelMode::Stereo);
        assert_eq!(st, vec![0.1, 0.9]);
        // 5.1 : FL seul → uniquement à gauche.
        let six = map_channels(&[1.0, 0.0, 0.0, 0.0, 0.0, 0.0], 6, ChannelMode::Stereo);
        assert!(six[0] > 0.0 && six[1] == 0.0);
    }
}
