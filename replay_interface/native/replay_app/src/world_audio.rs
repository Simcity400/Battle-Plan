//! Playback of derived world sound through the Windows waveform device.
//!
//! The law in [`crate::world_audio_law`] decides what the world sounds like;
//! this module owns only the output device, the wav decode, and the mixer.

use crate::world_audio_law::{
    battle_sites, cluster_battle_sites, BattleEmitter, BattleSoundCue, BattleSoundLaw,
    BattleSoundSilence, NSoundDefines, SoundBank,
};
use crate::Result;
use map_viewer::{province_anchors_from_resolver, GameFileResolver, ProvinceAnchors};
use std::collections::BTreeMap;
use std::sync::Arc;
use winapi::shared::minwindef::DWORD;
use winapi::shared::mmreg::{WAVEFORMATEX, WAVE_FORMAT_PCM};
use winapi::um::mmeapi::{
    waveOutClose, waveOutOpen, waveOutPrepareHeader, waveOutReset, waveOutUnprepareHeader,
    waveOutWrite,
};
use winapi::um::mmsystem::{
    HWAVEOUT, MMSYSERR_NOERROR, WAVEHDR, WAVE_MAPPER, CALLBACK_NULL,
};

/// The engine's wav corpus is entirely 44.1 kHz PCM, so the device runs at the
/// source rate and never resamples.
pub const DEVICE_SAMPLE_RATE: u32 = 44_100;
const DEVICE_CHANNELS: u16 = 2;
const BLOCK_FRAMES: usize = 1_024;
const BLOCK_COUNT: usize = 4;
/// `mmsystem.h`; winapi 0.3.9 does not re-export the WAVEHDR flags.
const WHDR_DONE: DWORD = 0x0000_0001;
/// Frames over which a gain change is walked, so starting, stopping, or moving
/// the camera cannot step the waveform and click. Device hygiene rather than a
/// law: no engine fade has been read out, which is one more reason the battle
/// voice stays silent (see `world_audio_law::BATTLE_GAIN_LAW_IS_PROVEN`).
const GAIN_GLIDE_FRAMES: f32 = 2_048.0;

/// One decoded wav, always interleaved stereo at [`DEVICE_SAMPLE_RATE`].
#[derive(Debug)]
pub struct Clip {
    frames: Vec<[i16; 2]>,
}

impl Clip {
    /// Decode a RIFF/PCM wav. Anything the engine's corpus does not contain is
    /// refused rather than approximated.
    pub fn decode(bytes: &[u8], source: &str) -> Result<Clip> {
        if bytes.len() < 12 || &bytes[0..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
            return Err(format!("world audio blocked: {source} is not a RIFF/WAVE file").into());
        }
        // The RIFF size counts everything after itself, so the form ends at
        // byte 8 + it. Trailing bytes past that end are not chunks.
        let riff_size =
            u32::from_le_bytes([bytes[4], bytes[5], bytes[6], bytes[7]]) as usize;
        let Some(riff_end) = riff_size.checked_add(8).filter(|end| *end <= bytes.len()) else {
            return Err(format!(
                "world audio blocked: {source} declares a {riff_size}-byte RIFF form in a {} \
                 byte file",
                bytes.len()
            )
            .into());
        };
        if riff_end < 12 {
            return Err(
                format!("world audio blocked: {source} declares an empty RIFF form").into(),
            );
        }
        let bytes = &bytes[..riff_end];
        let mut cursor = 12usize;
        let mut format: Option<(u16, u16, u32, u16, u16)> = None;
        let mut data: Option<&[u8]> = None;
        while cursor < bytes.len() {
            if cursor + 8 > bytes.len() {
                return Err(format!(
                    "world audio blocked: {source} ends inside a chunk header at byte {cursor}"
                )
                .into());
            }
            let id = &bytes[cursor..cursor + 4];
            let size = u32::from_le_bytes([
                bytes[cursor + 4],
                bytes[cursor + 5],
                bytes[cursor + 6],
                bytes[cursor + 7],
            ]) as usize;
            let body_start = cursor + 8;
            let Some(body_end) = body_start.checked_add(size).filter(|end| *end <= bytes.len())
            else {
                return Err(format!(
                    "world audio blocked: {source} declares a {size}-byte chunk at byte \
                     {body_start} that runs past the file"
                )
                .into());
            };
            if id == b"fmt " {
                if size < 16 {
                    return Err(format!(
                        "world audio blocked: {source} has a {size}-byte fmt chunk"
                    )
                    .into());
                }
                let body = &bytes[body_start..body_end];
                format = Some((
                    u16::from_le_bytes([body[0], body[1]]),
                    u16::from_le_bytes([body[2], body[3]]),
                    u32::from_le_bytes([body[4], body[5], body[6], body[7]]),
                    u16::from_le_bytes([body[12], body[13]]),
                    u16::from_le_bytes([body[14], body[15]]),
                ));
            } else if id == b"data" {
                data = Some(&bytes[body_start..body_end]);
            }
            let padded = body_end + (size & 1);
            if padded > bytes.len() {
                return Err(format!(
                    "world audio blocked: {source} has an odd-sized chunk at byte {body_start} \
                     with no RIFF pad byte"
                )
                .into());
            }
            cursor = padded;
        }
        let Some((tag, channels, rate, block_align, bits)) = format else {
            return Err(format!("world audio blocked: {source} has no fmt chunk").into());
        };
        let Some(data) = data else {
            return Err(format!("world audio blocked: {source} has no data chunk").into());
        };
        if tag != WAVE_FORMAT_PCM {
            return Err(
                format!("world audio blocked: {source} is wav format {tag}, not PCM").into(),
            );
        }
        if rate != DEVICE_SAMPLE_RATE {
            return Err(format!(
                "world audio blocked: {source} is {rate} Hz; the device runs at \
                 {DEVICE_SAMPLE_RATE} Hz and does not resample"
            )
            .into());
        }
        if channels != 1 && channels != 2 {
            return Err(
                format!("world audio blocked: {source} has {channels} channels").into(),
            );
        }
        let bytes_per_sample = match bits {
            16 => 2usize,
            24 => 3usize,
            other => {
                return Err(
                    format!("world audio blocked: {source} is {other}-bit PCM").into(),
                )
            }
        };
        let stride = bytes_per_sample * usize::from(channels);
        if usize::from(block_align) != stride {
            return Err(format!(
                "world audio blocked: {source} declares block align {block_align} for a \
                 {stride}-byte frame"
            )
            .into());
        }
        if data.len() % stride != 0 {
            return Err(format!(
                "world audio blocked: {source} has a {}-byte data chunk that is not a whole \
                 number of {stride}-byte frames",
                data.len()
            )
            .into());
        }
        let frame_count = data.len() / stride;
        let mut frames = Vec::with_capacity(frame_count);
        for index in 0..frame_count {
            let base = index * stride;
            let left = pcm_sample(&data[base..base + bytes_per_sample]);
            let right = if channels == 2 {
                pcm_sample(&data[base + bytes_per_sample..base + stride])
            } else {
                left
            };
            frames.push([left, right]);
        }
        if frames.is_empty() {
            return Err(format!("world audio blocked: {source} decodes to no frames").into());
        }
        Ok(Clip { frames })
    }
}

fn pcm_sample(raw: &[u8]) -> i16 {
    match raw.len() {
        2 => i16::from_le_bytes([raw[0], raw[1]]),
        // 24-bit takes the top two bytes: the discarded byte is below the
        // device's own 16-bit output resolution.
        3 => i16::from_le_bytes([raw[1], raw[2]]),
        _ => 0,
    }
}

struct Voice {
    clip: Arc<Clip>,
    cursor: usize,
    looping: bool,
    gain: f32,
    target_gain: f32,
    stopping: bool,
}

/// `WAVEHDR` is declared packed, so give the device a pointer the platform's
/// own header alignment would have had.
#[repr(C, align(8))]
struct AlignedHeader(WAVEHDR);

struct WaveOut {
    handle: HWAVEOUT,
    headers: Vec<Box<AlignedHeader>>,
    blocks: Vec<Vec<i16>>,
}

impl WaveOut {
    fn open() -> Result<WaveOut> {
        let format = WAVEFORMATEX {
            wFormatTag: WAVE_FORMAT_PCM,
            nChannels: DEVICE_CHANNELS,
            nSamplesPerSec: DEVICE_SAMPLE_RATE,
            nAvgBytesPerSec: DEVICE_SAMPLE_RATE * u32::from(DEVICE_CHANNELS) * 2,
            nBlockAlign: DEVICE_CHANNELS * 2,
            wBitsPerSample: 16,
            cbSize: 0,
        };
        let mut handle: HWAVEOUT = std::ptr::null_mut();
        let opened = unsafe {
            waveOutOpen(
                &mut handle,
                WAVE_MAPPER,
                &format,
                0,
                0,
                CALLBACK_NULL,
            )
        };
        if opened != MMSYSERR_NOERROR {
            return Err(format!("world audio device unavailable: waveOutOpen returned {opened}").into());
        }
        let samples = BLOCK_FRAMES * usize::from(DEVICE_CHANNELS);
        let mut blocks: Vec<Vec<i16>> = (0..BLOCK_COUNT).map(|_| vec![0i16; samples]).collect();
        let mut headers: Vec<Box<AlignedHeader>> = Vec::with_capacity(BLOCK_COUNT);
        for block in blocks.iter_mut() {
            let mut header: Box<AlignedHeader> =
                Box::new(AlignedHeader(unsafe { std::mem::zeroed() }));
            header.0.lpData = block.as_mut_ptr().cast();
            header.0.dwBufferLength = (samples * 2) as DWORD;
            let prepared = unsafe {
                waveOutPrepareHeader(
                    handle,
                    &mut header.0,
                    std::mem::size_of::<WAVEHDR>() as u32,
                )
            };
            if prepared != MMSYSERR_NOERROR {
                // Every header already prepared still belongs to the device.
                unsafe {
                    for done in headers.iter_mut() {
                        waveOutUnprepareHeader(
                            handle,
                            &mut (**done).0,
                            std::mem::size_of::<WAVEHDR>() as u32,
                        );
                    }
                    waveOutClose(handle);
                }
                return Err(format!(
                    "world audio device unavailable: waveOutPrepareHeader returned {prepared}"
                )
                .into());
            }
            header.0.dwFlags |= WHDR_DONE;
            headers.push(header);
        }
        Ok(WaveOut {
            handle,
            headers,
            blocks,
        })
    }

    /// Refill and requeue every block the device has finished with.
    fn pump(&mut self, voice: Option<&mut Voice>) -> Result<()> {
        let mut voice = voice;
        for (index, header) in self.headers.iter_mut().enumerate() {
            if header.0.dwFlags & WHDR_DONE == 0 {
                continue;
            }
            let block = &mut self.blocks[index];
            match voice.as_deref_mut() {
                Some(voice) => voice.render(block),
                None => block.fill(0),
            }
            header.0.dwFlags &= !WHDR_DONE;
            let written = unsafe {
                waveOutWrite(
                    self.handle,
                    &mut header.0,
                    std::mem::size_of::<WAVEHDR>() as u32,
                )
            };
            if written != MMSYSERR_NOERROR {
                header.0.dwFlags |= WHDR_DONE;
                return Err(format!(
                    "world audio device stalled: waveOutWrite returned {written}"
                )
                .into());
            }
        }
        Ok(())
    }
}

impl Drop for WaveOut {
    fn drop(&mut self) {
        unsafe {
            waveOutReset(self.handle);
            for header in self.headers.iter_mut() {
                waveOutUnprepareHeader(
                    self.handle,
                    &mut header.0,
                    std::mem::size_of::<WAVEHDR>() as u32,
                );
            }
            waveOutClose(self.handle);
        }
    }
}

impl Voice {
    fn render(&mut self, block: &mut [i16]) {
        let step = if self.stopping {
            -self.gain.max(0.0) / GAIN_GLIDE_FRAMES
        } else {
            (self.target_gain - self.gain) / GAIN_GLIDE_FRAMES
        };
        for frame in block.chunks_exact_mut(2) {
            self.gain = (self.gain + step).clamp(0.0, 4.0);
            if self.cursor >= self.clip.frames.len() {
                if !self.looping {
                    frame[0] = 0;
                    frame[1] = 0;
                    continue;
                }
                self.cursor = 0;
            }
            let source = self.clip.frames[self.cursor];
            self.cursor += 1;
            frame[0] = scale(source[0], self.gain);
            frame[1] = scale(source[1], self.gain);
        }
    }

    fn finished(&self) -> bool {
        (self.stopping && self.gain <= 0.0)
            || (!self.looping && self.cursor >= self.clip.frames.len())
    }
}

fn scale(sample: i16, gain: f32) -> i16 {
    (f32::from(sample) * gain).clamp(f32::from(i16::MIN), f32::from(i16::MAX)) as i16
}

/// What the world sounded like on one frame.
#[derive(Clone, Debug, PartialEq)]
pub struct WorldAudioFrame {
    pub cue: Option<BattleSoundCue>,
    pub silence: Option<BattleSoundSilence>,
    /// `None` when the hour was never placed, which is not the same as a
    /// measured zero.
    pub sites: Option<usize>,
    pub emitters: Option<usize>,
}

impl WorldAudioFrame {
    /// One line naming what plays, or exactly why nothing does.
    pub fn receipt(&self) -> String {
        let state = match (&self.cue, &self.silence) {
            (Some(cue), _) => format!(
                "effect={} sound={} wav={} provinces={:?} units={} distance={:.1} reach={:.1}(asset max_distance; engine falloff+0x24 unpinned) {}",
                cue.effect,
                cue.sound,
                cue.sound_path,
                cue.emitter.provinces,
                cue.emitter.units,
                cue.distance,
                cue.reach,
                match cue.gain {
                    Some(gain) => format!("gain={gain:.3}"),
                    None => "silent=gain-law-unread".to_string(),
                }
            ),
            (None, Some(silence)) => format!("silent={silence:?}"),
            (None, None) => "silent=unevaluated".to_string(),
        };
        let count = |value: Option<usize>| match value {
            Some(value) => value.to_string(),
            None => "unmeasured".to_string(),
        };
        format!(
            "audio battle {state} sites={} emitters={}",
            count(self.sites),
            count(self.emitters)
        )
    }
}

pub struct WorldAudio {
    law: BattleSoundLaw,
    anchors: ProvinceAnchors,
    /// Absent while the loudness law is unread: playback does not take an
    /// output device to play nothing.
    device: Option<WaveOut>,
    clips: BTreeMap<String, Arc<Clip>>,
    voice: Option<Voice>,
    playing: Option<String>,
    plays: usize,
    placed_hour: Option<i64>,
    placement: std::result::Result<Placement, BattleSoundSilence>,
}

#[derive(Clone, Debug, Default, PartialEq)]
struct Placement {
    sites: usize,
    emitters: Vec<BattleEmitter>,
}

impl WorldAudio {
    /// Bind the law to the recording's admitted content and open the device.
    pub fn bind(resolver: &GameFileResolver) -> Result<WorldAudio> {
        let bank = SoundBank::from_resolver(resolver)?;
        let law = BattleSoundLaw::bind(NSoundDefines::from_resolver(resolver)?, &bank)?;
        let anchors = province_anchors_from_resolver(resolver)?;
        eprintln!(
            "replay_app: derived world sound bound {} soundeffect(s) and {} sound(s) from {} \
             admitted asset file(s); battle effect {} falloff {:?}; {} province anchor(s); \
             voice {}",
            bank.effect_count(),
            bank.sound_count(),
            bank.sources().len(),
            law.defines.battle_sound_name,
            law.falloff,
            anchors.len(),
            if law.gain_law_proven {
                "audible"
            } else {
                "silent: the loudness law is unread"
            }
        );
        let device = if law.gain_law_proven {
            Some(WaveOut::open()?)
        } else {
            None
        };
        Ok(WorldAudio {
            law,
            anchors,
            device,
            clips: BTreeMap::new(),
            voice: None,
            playing: None,
            plays: 0,
            placed_hour: None,
            placement: Ok(Placement::default()),
        })
    }

    /// Advance the world's sound for one frame. `camera_eye` is the
    /// engine-space eye the frame was composed with. Placing and clustering the
    /// hour's combats is redone only when the displayed hour changes; the
    /// camera-dependent selection is redone every frame, as the engine does.
    pub fn update(
        &mut self,
        resolver: &GameFileResolver,
        hour: i64,
        combats: &[replay_core::CombatRow],
        combats_status: replay_core::ViewStatus,
        camera_eye: [f32; 3],
    ) -> Result<WorldAudioFrame> {
        if self.voice.as_ref().is_some_and(Voice::finished) {
            self.voice = None;
            self.playing = None;
        }
        if self.placed_hour != Some(hour) {
            self.placement = battle_sites(combats, combats_status, &self.anchors).map(|sites| {
                Placement {
                    sites: sites.len(),
                    emitters: cluster_battle_sites(
                        &sites,
                        self.law.defines.battle_sound_init_radius,
                    ),
                }
            });
            self.placed_hour = Some(hour);
        }
        // The engine picks one of the effect's sounds at random. Effect RNG is
        // owner-ruled viewer-local, so playback advances a deterministic
        // counter instead and stays reproducible.
        // A refused hour was never placed, so its counts are absent rather
        // than zero.
        let (sites, emitters, selected) = match &self.placement {
            Ok(placement) => (
                Some(placement.sites),
                Some(placement.emitters.len()),
                self.law.select(&placement.emitters, camera_eye, self.plays),
            ),
            Err(silence) => (None, None, Err(silence.clone())),
        };
        let frame = match selected {
            Ok(cue) => {
                // The wav is resolved and decoded either way, so the chain from
                // recorded state to real game audio is exercised on the shipping
                // path; only starting a voice waits on the loudness law.
                let clip = self.clip(resolver, &cue.sound_path)?;
                match cue.gain {
                    Some(gain) => self.start_or_retarget(&cue, clip, gain),
                    None => self.stop(),
                }
                WorldAudioFrame {
                    cue: Some(cue),
                    silence: None,
                    sites,
                    emitters,
                }
            }
            Err(silence) => {
                self.stop();
                WorldAudioFrame {
                    cue: None,
                    silence: Some(silence),
                    sites,
                    emitters,
                }
            }
        };
        if let Some(device) = self.device.as_mut() {
            device.pump(self.voice.as_mut())?;
        }
        Ok(frame)
    }

    fn start_or_retarget(&mut self, cue: &BattleSoundCue, clip: Arc<Clip>, gain: f32) {
        if self.playing.as_deref() == Some(cue.sound_path.as_str()) {
            if let Some(voice) = self.voice.as_mut() {
                voice.target_gain = gain;
                voice.stopping = false;
                return;
            }
        }
        self.voice = Some(Voice {
            clip,
            cursor: 0,
            looping: cue.looping,
            gain: 0.0,
            target_gain: gain,
            stopping: false,
        });
        self.playing = Some(cue.sound_path.clone());
        self.plays = self.plays.wrapping_add(1);
    }

    fn stop(&mut self) {
        if let Some(voice) = self.voice.as_mut() {
            voice.stopping = true;
        }
        self.playing = None;
    }

    fn clip(&mut self, resolver: &GameFileResolver, logical_path: &str) -> Result<Arc<Clip>> {
        if let Some(clip) = self.clips.get(logical_path) {
            return Ok(Arc::clone(clip));
        }
        let local = resolver.resolve_required(logical_path, "world sound payload")?;
        let bytes = std::fs::read(&local)?;
        let clip = Arc::new(Clip::decode(&bytes, logical_path)?);
        self.clips
            .insert(logical_path.to_string(), Arc::clone(&clip));
        Ok(clip)
    }
}

/// Playback's hold on the world-sound runtime.
///
/// Binding reads the province raster, so it waits for the first hour that has a
/// combat to place. A device or content failure silences the world with a named
/// reason and never stops playback.
pub enum WorldAudioSlot {
    Unbound,
    Ready(Box<WorldAudio>),
    Unavailable(String),
}

impl Default for WorldAudioSlot {
    fn default() -> Self {
        WorldAudioSlot::Unbound
    }
}

impl WorldAudioSlot {
    pub fn new() -> WorldAudioSlot {
        WorldAudioSlot::default()
    }

    pub fn update(
        &mut self,
        resolver: &GameFileResolver,
        hour: i64,
        combats: &[replay_core::CombatRow],
        combats_status: replay_core::ViewStatus,
        camera_eye: [f32; 3],
    ) -> Option<WorldAudioFrame> {
        if let WorldAudioSlot::Unavailable(reason) = self {
            return Some(unavailable_frame(reason.clone()));
        }
        if matches!(self, WorldAudioSlot::Unbound) {
            if combats_status != replay_core::ViewStatus::Present {
                return Some(WorldAudioFrame {
                    cue: None,
                    silence: Some(BattleSoundSilence::CombatsIncomplete(format!(
                        "combats view is {combats_status:?}"
                    ))),
                    sites: None,
                    emitters: None,
                });
            }
            if combats.is_empty() {
                // Binding reads the province raster, so it waits for an hour
                // that has something to place. The receipt still says why.
                return Some(WorldAudioFrame {
                    cue: None,
                    silence: Some(BattleSoundSilence::NoBattles),
                    // A Present view with no land combat is a measured zero.
                    sites: Some(0),
                    emitters: Some(0),
                });
            }
            match WorldAudio::bind(resolver) {
                Ok(audio) => *self = WorldAudioSlot::Ready(Box::new(audio)),
                Err(error) => return Some(self.give_up(format!("{error}"))),
            }
        }
        let outcome = match self {
            WorldAudioSlot::Ready(audio) => {
                audio.update(resolver, hour, combats, combats_status, camera_eye)
            }
            _ => unreachable!("the slot was just made ready"),
        };
        match outcome {
            Ok(frame) => Some(frame),
            Err(error) => Some(self.give_up(format!("{error}"))),
        }
    }

    fn give_up(&mut self, reason: String) -> WorldAudioFrame {
        eprintln!("replay_app: derived world sound is unavailable: {reason}");
        *self = WorldAudioSlot::Unavailable(reason.clone());
        unavailable_frame(reason)
    }
}

fn unavailable_frame(reason: String) -> WorldAudioFrame {
    WorldAudioFrame {
        cue: None,
        silence: Some(BattleSoundSilence::Unavailable(reason)),
        sites: None,
        emitters: None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn wav(channels: u16, bits: u16, rate: u32, samples: &[u8]) -> Vec<u8> {
        let block_align = channels * bits / 8;
        let mut out = Vec::new();
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(36u32 + samples.len() as u32).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&16u32.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&channels.to_le_bytes());
        out.extend_from_slice(&rate.to_le_bytes());
        out.extend_from_slice(&(rate * u32::from(block_align)).to_le_bytes());
        out.extend_from_slice(&block_align.to_le_bytes());
        out.extend_from_slice(&bits.to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&(samples.len() as u32).to_le_bytes());
        out.extend_from_slice(samples);
        out
    }

    #[test]
    fn mono_16_bit_is_duplicated_into_both_channels() {
        let bytes = wav(1, 16, DEVICE_SAMPLE_RATE, &[0x00, 0x40, 0x00, 0xc0]);
        let clip = Clip::decode(&bytes, "test.wav").expect("decode");
        assert_eq!(clip.frames, vec![[0x4000, 0x4000], [-0x4000, -0x4000]]);
    }

    #[test]
    fn stereo_24_bit_keeps_the_top_two_bytes_per_channel() {
        let bytes = wav(
            2,
            24,
            DEVICE_SAMPLE_RATE,
            &[0x11, 0x22, 0x33, 0x44, 0x55, 0x66],
        );
        let clip = Clip::decode(&bytes, "test.wav").expect("decode");
        assert_eq!(clip.frames, vec![[0x3322, 0x6655]]);
    }

    #[test]
    fn an_off_rate_or_non_pcm_wav_is_refused_not_resampled() {
        let resampled = Clip::decode(&wav(2, 16, 22_050, &[0, 0, 0, 0]), "off.wav")
            .expect_err("refusal");
        assert!(resampled.to_string().contains("does not resample"), "{resampled}");

        let mut adpcm = wav(2, 16, DEVICE_SAMPLE_RATE, &[0, 0, 0, 0]);
        adpcm[20] = 2;
        let error = Clip::decode(&adpcm, "adpcm.wav").expect_err("refusal");
        assert!(error.to_string().contains("not PCM"), "{error}");
    }

    #[test]
    fn a_truncated_or_empty_wav_is_refused() {
        assert!(Clip::decode(b"not a wav", "x.wav").is_err());
        let empty = Clip::decode(&wav(2, 16, DEVICE_SAMPLE_RATE, &[]), "empty.wav")
            .expect_err("refusal");
        assert!(empty.to_string().contains("no frames"), "{empty}");
    }

    #[test]
    fn a_data_chunk_that_overruns_the_file_is_refused_not_clamped() {
        let mut bytes = wav(2, 16, DEVICE_SAMPLE_RATE, &[1, 2, 3, 4]);
        // The data chunk size field sits at byte 40 of this header layout.
        bytes[40..44].copy_from_slice(&9_999u32.to_le_bytes());
        let error = Clip::decode(&bytes, "overrun.wav").expect_err("refusal");
        assert!(error.to_string().contains("runs past the file"), "{error}");
    }

    #[test]
    fn a_partial_final_frame_is_refused_not_discarded() {
        let bytes = wav(2, 16, DEVICE_SAMPLE_RATE, &[1, 2, 3, 4, 5, 6]);
        let error = Clip::decode(&bytes, "partial.wav").expect_err("refusal");
        assert!(
            error.to_string().contains("whole number of 4-byte frames"),
            "{error}"
        );
    }

    #[test]
    fn a_block_align_that_disagrees_with_the_frame_is_refused() {
        let mut bytes = wav(2, 16, DEVICE_SAMPLE_RATE, &[0, 0, 0, 0]);
        // wFormatTag is at 20, so nBlockAlign sits at 32.
        bytes[32..34].copy_from_slice(&8u16.to_le_bytes());
        let error = Clip::decode(&bytes, "align.wav").expect_err("refusal");
        assert!(error.to_string().contains("block align 8"), "{error}");
    }

    #[test]
    fn a_chunk_header_split_by_the_end_of_the_form_is_refused() {
        let mut bytes = wav(2, 16, DEVICE_SAMPLE_RATE, &[0, 0, 0, 0]);
        bytes.extend_from_slice(b"LIST");
        let riff = (bytes.len() - 8) as u32;
        bytes[4..8].copy_from_slice(&riff.to_le_bytes());
        let error = Clip::decode(&bytes, "split.wav").expect_err("refusal");
        assert!(
            error.to_string().contains("ends inside a chunk header"),
            "{error}"
        );
    }

    #[test]
    fn bytes_past_the_declared_riff_form_are_not_walked_as_chunks() {
        let mut bytes = wav(1, 16, DEVICE_SAMPLE_RATE, &[0x11, 0x22]);
        bytes.extend_from_slice(b"trailing garbage");
        let clip = Clip::decode(&bytes, "trailing.wav").expect("decode");
        assert_eq!(clip.frames, vec![[0x2211, 0x2211]]);
    }

    #[test]
    fn a_riff_size_larger_than_the_file_is_refused() {
        let mut bytes = wav(2, 16, DEVICE_SAMPLE_RATE, &[0, 0, 0, 0]);
        bytes[4..8].copy_from_slice(&9_999u32.to_le_bytes());
        let error = Clip::decode(&bytes, "shortfile.wav").expect_err("refusal");
        assert!(error.to_string().contains("RIFF form"), "{error}");
    }

    #[test]
    fn an_odd_sized_chunk_without_its_pad_byte_is_refused() {
        // One mono 24-bit frame: a three-byte data chunk that RIFF requires be
        // followed by a pad byte, and is not.
        let bytes = wav(1, 24, DEVICE_SAMPLE_RATE, &[0x11, 0x22, 0x33]);
        let error = Clip::decode(&bytes, "unpadded.wav").expect_err("refusal");
        assert!(error.to_string().contains("no RIFF pad byte"), "{error}");
    }

    #[test]
    fn a_leading_junk_chunk_before_fmt_is_walked_not_assumed_away() {
        // The shipped amb_battle_distant_02.wav carries JUNK and bext chunks
        // ahead of fmt; the decoder must find fmt by walking, not by offset.
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&0u32.to_le_bytes());
        bytes.extend_from_slice(b"WAVE");
        bytes.extend_from_slice(b"JUNK");
        bytes.extend_from_slice(&8u32.to_le_bytes());
        bytes.extend_from_slice(&[0u8; 8]);
        let tail = wav(1, 16, DEVICE_SAMPLE_RATE, &[0x11, 0x22]);
        bytes.extend_from_slice(&tail[12..]);
        let riff = (bytes.len() - 8) as u32;
        bytes[4..8].copy_from_slice(&riff.to_le_bytes());
        let clip = Clip::decode(&bytes, "junk.wav").expect("decode");
        assert_eq!(clip.frames, vec![[0x2211, 0x2211]]);
    }

    #[test]
    #[cfg_attr(
        not(gate_hoi4_install),
        ignore = "requires the matching the game install (set BATTLE_PLAN_GAME_DIR)"
    )]
    fn gate_vanilla_every_battle_wav_decodes_on_the_real_install() {
        use crate::world_audio_law::{BattleSoundLaw, NSoundDefines, SoundBank};
        let root = replay_test_gates::require_vanilla_game_root();
        let resolver = map_viewer::GameFileResolver::vanilla(root).expect("vanilla resolver");
        let bank = SoundBank::from_resolver(&resolver).expect("sound bank");
        let law = BattleSoundLaw::bind(
            NSoundDefines::from_resolver(&resolver).expect("NSound defines"),
            &bank,
        )
        .expect("battle sound law");
        assert!(!law.sounds.is_empty());
        for sound in &law.sounds {
            let path = resolver
                .resolve_required(&sound.path, "battle sound payload")
                .unwrap_or_else(|error| panic!("{}: {error}", sound.path));
            let bytes = std::fs::read(&path).expect("read battle sound payload");
            let clip = Clip::decode(&bytes, &sound.path)
                .unwrap_or_else(|error| panic!("{}: {error}", sound.path));
            assert!(clip.frames.len() > DEVICE_SAMPLE_RATE as usize / 10);
            eprintln!(
                "UCAUDIO gate: {} decoded {} frames at {DEVICE_SAMPLE_RATE} Hz",
                sound.path,
                clip.frames.len()
            );
        }
    }

    #[test]
    #[cfg_attr(
        not(gate_hoi4_install),
        ignore = "requires the matching the game install (set BATTLE_PLAN_GAME_DIR)"
    )]
    fn gate_vanilla_the_shipping_path_reaches_a_decoded_wav_and_takes_no_device() {
        use replay_core::{CombatRow, LandCombatDetail, Leaf, ViewStatus};
        let root = replay_test_gates::require_vanilla_game_root();
        let resolver = map_viewer::GameFileResolver::vanilla(root).expect("vanilla resolver");
        let mut audio = WorldAudio::bind(&resolver).expect("world audio");
        assert!(
            audio.device.is_none(),
            "no output device is taken while the loudness law is unread"
        );
        let province = (1u32..40_000)
            .find(|id| audio.anchors.anchor(*id).is_some())
            .expect("the raster paints at least one province");
        let anchor = *audio.anchors.anchor(province).expect("anchor");
        let combat = CombatRow {
            combat_id: Leaf::Measured(1),
            generation: Leaf::Measured(1),
            kind: Leaf::Measured("land".into()),
            location_province: Leaf::Measured(province),
            attackers: Leaf::Measured(vec![1, 2, 3, 4]),
            defenders: Leaf::Measured(vec![5, 6, 7, 8]),
            combat_width: Leaf::Measured(10.0),
            terrain: Leaf::Measured("plains".into()),
            progress: Leaf::Measured(0.0),
            detail: Leaf::<LandCombatDetail>::Unobtained,
        };
        let frame = audio
            .update(
                &resolver,
                1936,
                std::slice::from_ref(&combat),
                ViewStatus::Present,
                [anchor.x, 500.0, anchor.z],
            )
            .expect("world audio frame");
        let cue = frame.cue.as_ref().expect("the emitter is derived");
        assert_eq!(cue.emitter.provinces, vec![province]);
        assert_eq!(cue.emitter.units, 8);
        assert_eq!(cue.gain, None, "no loudness while the law is unread");
        assert!(audio.voice.is_none(), "nothing is queued to sound");
        let clip = audio
            .clips
            .get(&cue.sound_path)
            .expect("the real wav was resolved and decoded on the shipping path");
        assert!(clip.frames.len() > DEVICE_SAMPLE_RATE as usize / 10);
        eprintln!("UCAUDIO gate: {}", frame.receipt());
    }

    #[test]
    fn an_empty_hour_before_binding_separates_a_measured_lull_from_an_unavailable_view() {
        use replay_core::ViewStatus;
        let resolver = map_viewer::GameFileResolver::vanilla(std::env::temp_dir())
            .expect("a resolver over an empty root");
        for status in [ViewStatus::DomainAbsent, ViewStatus::DomainUnavailable] {
            let mut slot = WorldAudioSlot::new();
            let frame = slot
                .update(&resolver, 1936, &[], status, [0.0, 100.0, 0.0])
                .expect("a receipt");
            assert!(
                matches!(
                    frame.silence,
                    Some(BattleSoundSilence::CombatsIncomplete(_))
                ),
                "{status:?} must refuse, not report a lull: {frame:?}"
            );
            assert!(matches!(slot, WorldAudioSlot::Unbound));
            assert_eq!(frame.sites, None, "a refused hour counts nothing");
            assert_eq!(frame.emitters, None);
            assert!(frame.receipt().contains("sites=unmeasured"), "{}", frame.receipt());
        }
        let mut slot = WorldAudioSlot::new();
        let frame = slot
            .update(&resolver, 1936, &[], ViewStatus::Present, [0.0, 100.0, 0.0])
            .expect("a receipt");
        assert_eq!(frame.silence, Some(BattleSoundSilence::NoBattles));
        assert_eq!(frame.sites, Some(0), "a Present view with no combat is a measured zero");
        assert!(frame.receipt().contains("sites=0"), "{}", frame.receipt());
    }

    #[test]
    fn a_voice_glides_its_gain_and_loops_without_running_off_the_clip() {
        let clip = Arc::new(Clip {
            frames: vec![[1000, -1000], [2000, -2000]],
        });
        let mut voice = Voice {
            clip,
            cursor: 0,
            looping: true,
            gain: 1.0,
            target_gain: 1.0,
            stopping: false,
        };
        let mut block = vec![0i16; 8];
        voice.render(&mut block);
        assert_eq!(voice.cursor, 2, "four rendered frames wrapped a two-frame clip");
        assert!(block.iter().any(|sample| *sample != 0));
        assert!(!voice.finished());

        voice.stopping = true;
        for _ in 0..8 {
            voice.render(&mut block);
        }
        assert!(voice.gain < 1.0);
    }

    #[test]
    fn a_one_shot_voice_finishes_at_the_end_of_its_clip() {
        let clip = Arc::new(Clip {
            frames: vec![[1000, 1000]],
        });
        let mut voice = Voice {
            clip,
            cursor: 0,
            looping: false,
            gain: 1.0,
            target_gain: 1.0,
            stopping: false,
        };
        let mut block = vec![0i16; 8];
        voice.render(&mut block);
        assert!(voice.finished());
    }
}
