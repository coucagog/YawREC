// ============================================================
// YAWREC · encoder/mod.rs (E1 + E2 + E3bis)
// Wrapper d'encodage autour de `ffmpeg-next`.
//
// E1     : pipeline BGRA8 → sws → YUV420P → libx264 → mux MP4
// E2     : détection HW (NVENC, QSV, AMF) avec probing + fallback runtime
// E3 bis : stream audio AAC (48 kHz stéréo 128 kbps) dans le même MP4
//
// Threading : l'Encoder est conçu pour être possédé par un Arc<Mutex<>>.
// Les workers vidéo et audio prennent le lock brièvement pour pousser
// frame/samples. FFmpeg n'aime pas l'accès concurrent au même contexte —
// le Mutex sérialise les appels.
// ============================================================

use std::collections::VecDeque;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

use ffmpeg_next as ffmpeg;
use ffmpeg::{
    codec, encoder as ff_encoder, format, frame,
    software::scaling,
    util::{channel_layout::ChannelLayout, rational::Rational},
    Dictionary, Packet,
};

#[derive(Debug, thiserror::Error)]
pub enum EncoderError {
    #[error("Initialisation : {0}")]
    Init(String),
    #[error("Encodage : {0}")]
    Encoding(String),
    #[error("Mux : {0}")]
    Mux(String),
    #[error("FFmpeg : {0}")]
    Ffmpeg(#[from] ffmpeg::Error),
    #[error("E/S : {0}")]
    Io(#[from] std::io::Error),
}

// ============================================================
// VideoEncoder enum (E2)
// ============================================================

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum VideoEncoder {
    H264Nvenc,
    H264QuickSync,
    H264Amf,
    X264,
}

impl VideoEncoder {
    pub fn pick_best() -> Self {
        static CACHED: OnceLock<VideoEncoder> = OnceLock::new();
        *CACHED.get_or_init(|| {
            if let Ok(forced) = std::env::var("YAWREC_FORCE_ENCODER") {
                let parsed = match forced.as_str() {
                    "libx264"    => Some(VideoEncoder::X264),
                    "h264_nvenc" => Some(VideoEncoder::H264Nvenc),
                    "h264_qsv"   => Some(VideoEncoder::H264QuickSync),
                    "h264_amf"   => Some(VideoEncoder::H264Amf),
                    _ => None,
                };
                match parsed {
                    Some(enc) => {
                        log::info!("YAWREC_FORCE_ENCODER={forced} → override actif");
                        return enc;
                    }
                    None => {
                        log::warn!(
                            "YAWREC_FORCE_ENCODER={forced} ignoré \
                             (valeurs valides : libx264, h264_nvenc, h264_qsv, h264_amf)",
                        );
                    }
                }
            }

            log::info!("Détection encodeur vidéo…");
            for candidate in [
                VideoEncoder::H264Nvenc,
                VideoEncoder::H264QuickSync,
                VideoEncoder::H264Amf,
            ] {
                match probe_encoder(candidate) {
                    Ok(()) => {
                        log::info!("probe[{}] : OK", candidate.ffmpeg_name());
                        log::info!(
                            "Encodeur sélectionné : {} (matériel)",
                            candidate.ffmpeg_name(),
                        );
                        return candidate;
                    }
                    Err(e) => log::debug!("probe[{}] : {e}", candidate.ffmpeg_name()),
                }
            }
            log::info!("probe[libx264] : OK");
            log::info!("Encodeur sélectionné : libx264 (logiciel)");
            VideoEncoder::X264
        })
    }

    pub fn ffmpeg_name(self) -> &'static str {
        match self {
            VideoEncoder::H264Nvenc     => "h264_nvenc",
            VideoEncoder::H264QuickSync => "h264_qsv",
            VideoEncoder::H264Amf       => "h264_amf",
            VideoEncoder::X264          => "libx264",
        }
    }

    pub fn display_name(self) -> &'static str {
        match self {
            VideoEncoder::H264Nvenc     => "NVIDIA NVENC (H.264)",
            VideoEncoder::H264QuickSync => "Intel QuickSync (H.264)",
            VideoEncoder::H264Amf       => "AMD AMF (H.264)",
            VideoEncoder::X264          => "x264 software (H.264)",
        }
    }

    pub fn is_hardware(self) -> bool {
        !matches!(self, VideoEncoder::X264)
    }

    fn encoder_options(self) -> Vec<(&'static str, &'static str)> {
        match self {
            VideoEncoder::X264 => vec![
                ("preset",  "veryfast"),
                ("profile", "high"),
            ],
            VideoEncoder::H264Nvenc => vec![
                ("preset",  "p4"),
                ("tune",    "ll"),
                ("profile", "high"),
                ("rc",      "vbr"),
            ],
            VideoEncoder::H264QuickSync => vec![
                ("preset",  "veryfast"),
                ("profile", "high"),
            ],
            VideoEncoder::H264Amf => vec![
                ("usage",   "lowlatency"),
                ("quality", "balanced"),
                ("profile", "high"),
            ],
        }
    }
}

fn probe_encoder(enc: VideoEncoder) -> Result<(), String> {
    ffmpeg::init().map_err(|e| format!("ffmpeg::init : {e}"))?;

    let codec = ff_encoder::find_by_name(enc.ffmpeg_name())
        .ok_or_else(|| format!("codec {} non compilé dans FFmpeg", enc.ffmpeg_name()))?;

    let context = codec::context::Context::new_with_codec(codec);
    let mut probe = context.encoder().video()
        .map_err(|e| format!("context vidéo : {e}"))?;

    probe.set_width(1280);
    probe.set_height(720);
    probe.set_format(format::Pixel::YUV420P);
    probe.set_time_base(Rational(1, 30));
    probe.set_frame_rate(Some(Rational(30, 1)));
    probe.set_bit_rate(2_000_000);

    let _opened = probe.open().map_err(|e| format!("open : {e}"))?;
    Ok(())
}

// ============================================================
// Config
// ============================================================

#[derive(Debug, Clone)]
pub struct EncoderConfig {
    pub output_path: PathBuf,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub bitrate_kbps: u32,
    pub video_encoder: VideoEncoder,

    // Audio (E3bis)
    pub with_audio: bool,
    pub audio_sample_rate: u32,
    pub audio_channels: u16,
    pub audio_bitrate_kbps: u32,
}

impl Default for EncoderConfig {
    fn default() -> Self {
        Self {
            output_path: PathBuf::from("output.mp4"),
            width: 1920,
            height: 1080,
            fps: 30,
            bitrate_kbps: 8000,
            video_encoder: VideoEncoder::pick_best(),
            with_audio: true,
            audio_sample_rate: 48000,
            audio_channels: 2,
            audio_bitrate_kbps: 128,
        }
    }
}

// ============================================================
// Encoder
// ============================================================

struct VideoTrack {
    encoder: ff_encoder::Video,
    scaler: scaling::Context,
    bgra_frame: frame::Video,
    yuv_frame: frame::Video,
    frame_index: i64,
    stream_idx: usize,
    used_encoder: VideoEncoder,
}

struct AudioTrack {
    encoder: ff_encoder::Audio,
    /// Buffer interleavé en F32 stéréo @48kHz, accumule jusqu'à frame_size*2.
    sample_buffer: VecDeque<f32>,
    frame_size: usize,    // samples par canal (généralement 1024 pour AAC)
    pts: i64,             // PTS en samples (time_base = 1/sample_rate)
    stream_idx: usize,
}

pub struct Encoder {
    octx: format::context::Output,
    video: VideoTrack,
    audio: Option<AudioTrack>,

    width: u32,
    height: u32,
    fps: u32,
    audio_sample_rate: u32,

    bytes_written: u64,
    output_path: PathBuf,
    finished: bool,
}

impl Encoder {
    pub fn new(config: EncoderConfig) -> Result<Self, EncoderError> {
        ffmpeg::init().map_err(EncoderError::Ffmpeg)?;

        if let Some(parent) = config.output_path.parent() {
            if !parent.as_os_str().is_empty() && !parent.exists() {
                std::fs::create_dir_all(parent)?;
            }
        }

        let mut octx = format::output(&config.output_path)
            .map_err(EncoderError::Ffmpeg)?;
        let global_header = octx.format()
            .flags()
            .contains(format::flag::Flags::GLOBAL_HEADER);

        // ---- Track vidéo ----
        let (video_encoder, used_video) =
            open_video_encoder(config.video_encoder, &config, global_header)?;
        let video_stream_idx = add_stream_with_params(&mut octx, &video_encoder, used_video.ffmpeg_name(), config.fps as i32)?;

        let bgra_frame = frame::Video::new(format::Pixel::BGRA, config.width, config.height);
        let yuv_frame  = frame::Video::new(format::Pixel::YUV420P, config.width, config.height);
        let scaler = scaling::Context::get(
            format::Pixel::BGRA,    config.width, config.height,
            format::Pixel::YUV420P, config.width, config.height,
            scaling::Flags::BILINEAR,
        ).map_err(EncoderError::Ffmpeg)?;

        let video = VideoTrack {
            encoder: video_encoder,
            scaler,
            bgra_frame,
            yuv_frame,
            frame_index: 0,
            stream_idx: video_stream_idx,
            used_encoder: used_video,
        };

        // ---- Track audio (E3bis) ----
        let audio = if config.with_audio {
            match open_audio_track(&mut octx, &config, global_header) {
                Ok(t) => Some(t),
                Err(e) => {
                    // Audio optionnel : on log et on continue en vidéo-seule.
                    log::warn!("Audio désactivé : {e}");
                    None
                }
            }
        } else {
            None
        };

        // ---- Écrire le header (après que tous les streams soient ajoutés) ----
        octx.write_header().map_err(EncoderError::Ffmpeg)?;

        log::info!(
            "Encoder · {}x{}@{}fps + {} → {} ({}, {} kbps vidéo{})",
            config.width, config.height, config.fps,
            if audio.is_some() { "audio AAC" } else { "(silencieux)" },
            config.output_path.display(),
            used_video.display_name(),
            config.bitrate_kbps,
            if audio.is_some() {
                format!(", {} kbps audio", config.audio_bitrate_kbps)
            } else { String::new() },
        );

        Ok(Self {
            octx,
            video,
            audio,
            width: config.width,
            height: config.height,
            fps: config.fps,
            audio_sample_rate: config.audio_sample_rate,
            bytes_written: 0,
            output_path: config.output_path,
            finished: false,
        })
    }

    // ---------------------------------------------------------
    // Vidéo
    // ---------------------------------------------------------
    pub fn push_video_frame(&mut self, bgra: &[u8], timestamp_ms: u64) -> Result<(), EncoderError> {
        if self.finished {
            return Err(EncoderError::Encoding("Encoder déjà fermé".into()));
        }

        let stride    = self.video.bgra_frame.stride(0);
        let row_bytes = (self.width as usize) * 4;
        let expected  = row_bytes * self.height as usize;

        if bgra.len() < expected {
            return Err(EncoderError::Encoding(format!(
                "Buffer vidéo trop petit : {} < {}", bgra.len(), expected
            )));
        }

        let dst = self.video.bgra_frame.data_mut(0);
        if stride == row_bytes {
            dst[..expected].copy_from_slice(&bgra[..expected]);
        } else {
            for y in 0..self.height as usize {
                let src_off = y * row_bytes;
                let dst_off = y * stride;
                dst[dst_off..dst_off + row_bytes]
                    .copy_from_slice(&bgra[src_off..src_off + row_bytes]);
            }
        }

        self.video.scaler.run(&self.video.bgra_frame, &mut self.video.yuv_frame)
            .map_err(EncoderError::Ffmpeg)?;

        self.video.yuv_frame.set_pts(Some(timestamp_ms as i64));
        self.video.frame_index += 1;

        self.video.encoder.send_frame(&self.video.yuv_frame)
            .map_err(EncoderError::Ffmpeg)?;

        self.drain_video_packets()?;
        Ok(())
    }

    fn drain_video_packets(&mut self) -> Result<(), EncoderError> {
        let enc_tb = self.video.encoder.time_base();
        let stream_tb = self.octx
            .stream(self.video.stream_idx)
            .ok_or_else(|| EncoderError::Mux("stream vidéo introuvable".into()))?
            .time_base();

        loop {
            let mut packet = Packet::empty();
            match self.video.encoder.receive_packet(&mut packet) {
                Ok(()) => {
                    packet.set_stream(self.video.stream_idx);
                    packet.rescale_ts(enc_tb, stream_tb);
                    self.bytes_written += packet.size() as u64;
                    packet.write_interleaved(&mut self.octx)
                        .map_err(EncoderError::Ffmpeg)?;
                }
                Err(_) => break,
            }
        }
        Ok(())
    }

    // ---------------------------------------------------------
    // Audio (E3bis)
    // ---------------------------------------------------------

    /// `samples` : interleavé stéréo f32 @ self.audio_sample_rate.
    /// Si aucun track audio n'a été configuré, l'appel est ignoré silencieusement.
    pub fn push_audio_samples(&mut self, samples: &[f32], _timestamp_ms: u64) -> Result<(), EncoderError> {
        if self.finished {
            return Err(EncoderError::Encoding("Encoder déjà fermé".into()));
        }

        let audio = match self.audio.as_mut() {
            Some(a) => a,
            None => return Ok(()),
        };

        // Empile dans le buffer (f32 interleavé stéréo)
        audio.sample_buffer.extend(samples.iter().copied());

        let needed = audio.frame_size * 2; // stéréo
        while audio.sample_buffer.len() >= needed {
            // Drain `needed` samples dans un Vec contigu (pour cast en bytes)
            let chunk: Vec<f32> = audio.sample_buffer
                .drain(..needed)
                .collect();

            // De-interleave LRLRLR → planar [LL…] [RR…] manuellement.
            // Le resampler swresample packed→planar produisait un canal R vide
            // sur certains devices WASAPI (ex. C270 en stéréo-WASAPI).
            let mut planar = frame::Audio::new(
                format::Sample::F32(format::sample::Type::Planar),
                audio.frame_size,
                ChannelLayout::STEREO,
            );
            planar.set_rate(self.audio_sample_rate);

            // SAFETY : f32 est trivially copyable, les slices sont correctement
            // dimensionnés par ffmpeg-next (frame_size * 4 octets par plan).
            unsafe {
                let dst_l = planar.data_mut(0).as_mut_ptr() as *mut f32;
                let dst_r = planar.data_mut(1).as_mut_ptr() as *mut f32;
                for i in 0..audio.frame_size {
                    *dst_l.add(i) = chunk[i * 2];
                    *dst_r.add(i) = chunk[i * 2 + 1];
                }
            }

            planar.set_pts(Some(audio.pts));
            audio.pts += audio.frame_size as i64;

            audio.encoder.send_frame(&planar).map_err(EncoderError::Ffmpeg)?;
            drain_audio_packets(audio, &mut self.octx, &mut self.bytes_written)?;
        }
        Ok(())
    }

    // ---------------------------------------------------------
    // Stop / flush
    // ---------------------------------------------------------
    pub fn stop(&mut self) -> Result<PathBuf, EncoderError> {
        if self.finished {
            return Ok(self.output_path.clone());
        }
        log::info!("Encoder · flush + close");

        // Flush vidéo
        self.video.encoder.send_eof().map_err(EncoderError::Ffmpeg)?;
        self.drain_video_packets()?;

        // Flush audio
        if let Some(audio) = self.audio.as_mut() {
            audio.encoder.send_eof().map_err(EncoderError::Ffmpeg)?;
            drain_audio_packets(audio, &mut self.octx, &mut self.bytes_written)?;
        }

        self.octx.write_trailer().map_err(EncoderError::Ffmpeg)?;
        self.finished = true;

        log::info!(
            "Encoder · MP4 fermé — {} frames vidéo, {} octets, codec {}",
            self.video.frame_index, self.bytes_written, self.video.used_encoder.ffmpeg_name(),
        );
        Ok(self.output_path.clone())
    }

    pub fn bytes_written(&self) -> u64 { self.bytes_written }
    pub fn frame_count(&self)   -> u64 { self.video.frame_index.max(0) as u64 }
    pub fn output_path(&self)   -> &Path { &self.output_path }
    pub fn used_encoder(&self)  -> VideoEncoder { self.video.used_encoder }
    pub fn has_audio(&self)     -> bool { self.audio.is_some() }
}

// SAFETY: Encoder is always accessed through Arc<Mutex<Option<Encoder>>>,
// which serialises all access. The non-Send field is scaling::Context
// (*mut SwsContext), which ffmpeg-next 8 no longer marks Send on its own.
unsafe impl Send for Encoder {}

impl Drop for Encoder {
    fn drop(&mut self) {
        if !self.finished {
            log::warn!(
                "Encoder droppé sans stop() — tentative de flush du fichier '{}'",
                self.output_path.display(),
            );
            let _ = self.video.encoder.send_eof();
            let _ = self.drain_video_packets();
            if let Some(audio) = self.audio.as_mut() {
                let _ = audio.encoder.send_eof();
                let _ = drain_audio_packets(audio, &mut self.octx, &mut self.bytes_written);
            }
            let _ = self.octx.write_trailer();
        }
    }
}

// ============================================================
// Helpers
// ============================================================

fn drain_audio_packets(
    audio: &mut AudioTrack,
    octx: &mut format::context::Output,
    bytes_written: &mut u64,
) -> Result<(), EncoderError> {
    let enc_tb = audio.encoder.time_base();
    let stream_tb = octx
        .stream(audio.stream_idx)
        .ok_or_else(|| EncoderError::Mux("stream audio introuvable".into()))?
        .time_base();

    loop {
        let mut packet = Packet::empty();
        match audio.encoder.receive_packet(&mut packet) {
            Ok(()) => {
                packet.set_stream(audio.stream_idx);
                packet.rescale_ts(enc_tb, stream_tb);
                *bytes_written += packet.size() as u64;
                packet.write_interleaved(octx).map_err(EncoderError::Ffmpeg)?;
            }
            Err(_) => break,
        }
    }
    Ok(())
}

/// Crée le stream dans l'output context et copie les paramètres depuis l'encoder.
fn add_stream_with_params(
    octx: &mut format::context::Output,
    encoder: &ff_encoder::Video,
    codec_name: &str,
    _fps: i32,
) -> Result<usize, EncoderError> {
    let codec = ff_encoder::find_by_name(codec_name)
        .ok_or_else(|| EncoderError::Init(format!("codec {codec_name} introuvable")))?;
    let mut stream = octx.add_stream(codec).map_err(EncoderError::Ffmpeg)?;
    stream.set_parameters(encoder);
    stream.set_time_base(Rational(1, 1000));
    Ok(stream.index())
}

fn open_video_encoder(
    candidate: VideoEncoder,
    config: &EncoderConfig,
    global_header: bool,
) -> Result<(ff_encoder::Video, VideoEncoder), EncoderError> {
    match try_open_video(candidate, config, global_header) {
        Ok(opened) => Ok((opened, candidate)),
        Err(e) if candidate != VideoEncoder::X264 => {
            log::warn!(
                "Ouverture {} a échoué ({e}) — fallback x264",
                candidate.ffmpeg_name(),
            );
            let opened = try_open_video(VideoEncoder::X264, config, global_header)?;
            Ok((opened, VideoEncoder::X264))
        }
        Err(e) => Err(e),
    }
}

fn try_open_video(
    which: VideoEncoder,
    config: &EncoderConfig,
    global_header: bool,
) -> Result<ff_encoder::Video, EncoderError> {
    let codec = ff_encoder::find_by_name(which.ffmpeg_name())
        .ok_or_else(|| EncoderError::Init(format!(
            "Encoder '{}' introuvable", which.ffmpeg_name()
        )))?;

    let context = codec::context::Context::new_with_codec(codec);
    let mut enc = context.encoder().video().map_err(EncoderError::Ffmpeg)?;

    enc.set_width(config.width);
    enc.set_height(config.height);
    enc.set_format(format::Pixel::YUV420P);
    enc.set_time_base(Rational(1, 1000));
    enc.set_frame_rate(Some(Rational(config.fps as i32, 1)));
    enc.set_bit_rate((config.bitrate_kbps as usize) * 1000);
    enc.set_max_bit_rate((config.bitrate_kbps as usize) * 1500);
    enc.set_gop(config.fps * 2);

    if global_header {
        enc.set_flags(codec::flag::Flags::GLOBAL_HEADER);
    }

    let mut opts = Dictionary::new();
    for (k, v) in which.encoder_options() {
        opts.set(k, v);
    }

    enc.open_with(opts).map_err(EncoderError::Ffmpeg)
}

/// Configure le track audio AAC. Si AAC n'est pas dispo dans le build FFmpeg
/// ou si l'ouverture échoue, retourne une erreur — l'appelant choisira de
/// désactiver l'audio ou de remonter l'erreur.
fn open_audio_track(
    octx: &mut format::context::Output,
    config: &EncoderConfig,
    global_header: bool,
) -> Result<AudioTrack, EncoderError> {
    let codec = ff_encoder::find(codec::Id::AAC)
        .ok_or_else(|| EncoderError::Init("codec AAC absent du build FFmpeg".into()))?;

    let context = codec::context::Context::new_with_codec(codec);
    let mut enc = context.encoder().audio().map_err(EncoderError::Ffmpeg)?;

    let layout = if config.audio_channels == 2 {
        ChannelLayout::STEREO
    } else {
        ChannelLayout::MONO
    };

    enc.set_rate(config.audio_sample_rate as i32);
    enc.set_channel_layout(layout);
    enc.set_format(format::Sample::F32(format::sample::Type::Planar));
    enc.set_bit_rate((config.audio_bitrate_kbps as usize) * 1000);
    enc.set_time_base(Rational(1, config.audio_sample_rate as i32));

    if global_header {
        enc.set_flags(codec::flag::Flags::GLOBAL_HEADER);
    }

    let opened = enc.open().map_err(EncoderError::Ffmpeg)?;
    let frame_size = if opened.frame_size() > 0 { opened.frame_size() as usize } else { 1024 };

    // Ajoute le stream et copie les paramètres
    let mut stream = octx.add_stream(codec).map_err(EncoderError::Ffmpeg)?;
    stream.set_parameters(&opened);
    stream.set_time_base(Rational(1, config.audio_sample_rate as i32));
    let stream_idx = stream.index();

    log::info!(
        "Audio track · AAC @{}Hz stéréo, frame_size={}, {} kbps",
        config.audio_sample_rate, frame_size, config.audio_bitrate_kbps,
    );

    Ok(AudioTrack {
        encoder: opened,
        sample_buffer: VecDeque::with_capacity(frame_size * 4),
        frame_size,
        pts: 0,
        stream_idx,
    })
}
