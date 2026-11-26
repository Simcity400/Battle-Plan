pub mod color;
pub mod ffmpeg;
#[cfg(windows)]
pub mod mf;
pub mod mp4_boxes;
pub mod schedule;

use serde::{Deserialize, Serialize};
use std::{
    io,
    path::{Path, PathBuf},
};

pub const AUDIO_SAMPLE_RATE: u32 = 44_100;
pub const AUDIO_CHANNELS: u16 = 2;
pub const DEFAULT_AUDIO_BITRATE: u32 = 192_000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum Quality {
    Standard,
    #[default]
    High,
    Maximum,
}

impl Quality {
    pub fn ffmpeg_crf(self) -> u32 {
        match self {
            Self::Standard => 23,
            Self::High => 18,
            Self::Maximum => 14,
        }
    }

    pub fn target_bitrate(self, width: u32, height: u32, fps: u32) -> u32 {
        let pixels = f64::from(width) * f64::from(height);
        let scale = (pixels / (1920.0 * 1080.0)).max(0.25);
        let fps_scale = (f64::from(fps) / 60.0).clamp(0.5, 2.0);
        let base_1080p60 = match self {
            Self::Standard => 10_000_000.0,
            Self::High => 18_000_000.0,
            Self::Maximum => 32_000_000.0,
        };
        (base_1080p60 * scale * fps_scale).clamp(1_000_000.0, 120_000_000.0) as u32
    }
}

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Deserialize, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum BackendChoice {
    #[default]
    Auto,
    MediaFoundation,
    Ffmpeg,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct VideoConfig {
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub quality: Quality,
    pub backend: BackendChoice,
    pub ffmpeg_path: Option<PathBuf>,
}

impl VideoConfig {
    pub fn validate(&self) -> io::Result<()> {
        if self.width < 2 || self.height < 2 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "Video width and height must each be at least 2 pixels",
            ));
        }
        if self.width % 2 != 0 || self.height % 2 != 0 {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "H.264 video requires even dimensions, got {}x{}",
                    self.width, self.height
                ),
            ));
        }
        if !(1..=240).contains(&self.fps) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!("Unsupported video frame rate: {} fps", self.fps),
            ));
        }
        Ok(())
    }

    pub fn rgba_frame_bytes(&self) -> io::Result<usize> {
        usize::try_from(self.width)
            .ok()
            .and_then(|w| usize::try_from(self.height).ok().and_then(|h| w.checked_mul(h)))
            .and_then(|pixels| pixels.checked_mul(4))
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "Video frame size overflows memory"))
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AudioConfig {
    pub sample_rate: u32,
    pub channels: u16,
    pub bitrate: u32,
}

impl Default for AudioConfig {
    fn default() -> Self {
        Self {
            sample_rate: AUDIO_SAMPLE_RATE,
            channels: AUDIO_CHANNELS,
            bitrate: DEFAULT_AUDIO_BITRATE,
        }
    }
}

impl AudioConfig {
    pub fn validate(&self) -> io::Result<()> {
        if self.sample_rate != AUDIO_SAMPLE_RATE || self.channels != AUDIO_CHANNELS {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                format!(
                    "Audio export requires 44,100 Hz stereo PCM, got {} Hz {} ch",
                    self.sample_rate, self.channels
                ),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BackendAvailability {
    pub media_foundation: Result<String, String>,
    pub ffmpeg: Result<ffmpeg::FfmpegInfo, String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Finished {
    pub path: PathBuf,
    pub frames: u64,
    pub audio_frames: u64,
    pub backend: String,
}

pub trait VideoSink: Send {
    fn push_video_rgba(&mut self, rgba: &[u8]) -> io::Result<()>;
    fn push_audio_stereo_i16(&mut self, pcm: &[i16]) -> io::Result<()>;
    fn frames_written(&self) -> u64;
    fn audio_frames_written(&self) -> u64;
    fn backend_name(&self) -> &str;
    fn finish(self: Box<Self>) -> io::Result<Finished>;
    fn abort(self: Box<Self>);
}

pub fn probe_backends(ffmpeg_path: Option<&Path>) -> BackendAvailability {
    #[cfg(windows)]
    let media_foundation = mf::probe();
    #[cfg(not(windows))]
    let media_foundation = Err("Windows Media Foundation is only available on Windows".to_owned());
    let ffmpeg = ffmpeg::probe(ffmpeg_path).map_err(|error| error.to_string());
    BackendAvailability {
        media_foundation,
        ffmpeg,
    }
}

pub fn open(
    config: &VideoConfig,
    audio: Option<AudioConfig>,
    staged_path: &Path,
) -> io::Result<Box<dyn VideoSink>> {
    config.validate()?;
    if let Some(audio) = audio {
        audio.validate()?;
    }
    match config.backend {
        BackendChoice::MediaFoundation => open_mf(config, audio, staged_path),
        BackendChoice::Ffmpeg => ffmpeg::FfmpegSink::open(config, audio, staged_path)
            .map(|sink| Box::new(sink) as Box<dyn VideoSink>),
        BackendChoice::Auto => {
            #[cfg(windows)]
            {
                match open_mf(config, audio, staged_path) {
                    Ok(sink) => Ok(sink),
                    Err(mf_error) => match ffmpeg::FfmpegSink::open(config, audio, staged_path) {
                        Ok(sink) => Ok(Box::new(sink) as Box<dyn VideoSink>),
                        Err(ff_error) => Err(io::Error::other(format!(
                            "No working MP4 encoder is available. Media Foundation: {mf_error}. FFmpeg: {ff_error}"
                        ))),
                    },
                }
            }
            #[cfg(not(windows))]
            {
                ffmpeg::FfmpegSink::open(config, audio, staged_path)
                    .map(|sink| Box::new(sink) as Box<dyn VideoSink>)
            }
        }
    }
}

fn open_mf(
    config: &VideoConfig,
    audio: Option<AudioConfig>,
    staged_path: &Path,
) -> io::Result<Box<dyn VideoSink>> {
    #[cfg(windows)]
    {
        mf::MfSink::open(config, audio, staged_path).map(|sink| Box::new(sink) as Box<dyn VideoSink>)
    }
    #[cfg(not(windows))]
    {
        let _ = (config, audio, staged_path);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "Windows Media Foundation is only available on Windows; select FFmpeg instead",
        ))
    }
}

pub fn snap_even(value: u32) -> u32 {
    (value.max(2) + 1) & !1
}
