//! RIFF WAVE sample files as the game's audio loader opens them.
//!
//! The game opens samples with SDL's WAVE loader, which hands over linear PCM
//! at 8 bits unsigned, 16 bits, and 24 or 32 bits widened to 32, IEEE float
//! at 32 bits, and A-law, mu-law and ADPCM decoded to 16 bits. [`decode`]
//! returns the same samples with the [`Format`] the loader reports, as
//! interleaved `f32` values exactly as SDL's converter reads that format
//! into float. IMA and Microsoft ADPCM decode through Symphonia. Other
//! encodings (64-bit float among them, which the loader rejects) and more
//! than two channels are refused by name: [`Refusal`] names the form so
//! consumers can count it.

use std::fmt;

const FORMAT_PCM: u16 = 1;
const FORMAT_FLOAT: u16 = 3;
const FORMAT_ALAW: u16 = 6;
const FORMAT_MULAW: u16 = 7;
const FORMAT_EXTENSIBLE: u16 = 0xfffe;
const FORMAT_MS_ADPCM: u16 = 2;
const FORMAT_IMA_ADPCM: u16 = 0x11;

/// SDL's float scale for 16-bit samples.
pub const DIVBY32768: f32 = 1.0 / 32_768.0;
/// SDL's float scale for 8-bit samples.
const DIVBY128: f32 = 1.0 / 128.0;
/// SDL's float scale for 32-bit samples after their low byte is dropped.
const DIVBY8388607: f32 = 0.000_000_119_209_3;

/// The sample format SDL's WAVE loader reports for a file.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    U8,
    S16,
    /// 24-bit samples widened to 32 bits, and 32-bit samples.
    S32,
    F32,
}

/// Decoded samples, interleaved by channel, as SDL's converter reads the
/// loader's [`Format`] into float.
#[derive(Clone, Debug, PartialEq)]
pub struct Pcm {
    pub rate: u32,
    pub channels: u16,
    pub format: Format,
    pub samples: Vec<f32>,
}

impl Pcm {
    pub fn frames(&self) -> usize {
        self.samples.len() / usize::from(self.channels.max(1))
    }

    /// The frame's left and right samples; mono plays on both sides.
    pub fn frame(&self, index: usize) -> [f32; 2] {
        match self.channels {
            1 => {
                let value = self.samples[index];
                [value, value]
            }
            _ => {
                let base = index * usize::from(self.channels);
                [self.samples[base], self.samples[base + 1]]
            }
        }
    }
}

/// Why a file is not decoded.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    NotWave,
    Truncated,
    MissingFormat,
    MissingData,
    /// The format tag and bits per sample of an unsupported encoding.
    Encoding {
        tag: u16,
        bits: u16,
    },
    /// An ADPCM stream the decoder refuses.
    Adpcm(String),
    Channels(u16),
    Rate(u32),
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotWave => write!(f, "not a RIFF WAVE file"),
            Self::Truncated => write!(f, "truncated WAVE chunk"),
            Self::MissingFormat => write!(f, "WAVE file without a format chunk"),
            Self::MissingData => write!(f, "WAVE file without a data chunk"),
            Self::Encoding { tag, bits } => {
                write!(f, "unsupported WAVE encoding {tag:#06x} at {bits} bits")
            }
            Self::Adpcm(why) => write!(f, "undecodable ADPCM WAVE ({why})"),
            Self::Channels(channels) => write!(f, "unsupported WAVE channel count {channels}"),
            Self::Rate(rate) => write!(f, "unsupported WAVE sample rate {rate}"),
        }
    }
}

struct FormatChunk {
    tag: u16,
    channels: u16,
    rate: u32,
    block: u16,
    bits: u16,
}

pub fn decode(bytes: &[u8]) -> Result<Pcm, Refusal> {
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(Refusal::NotWave);
    }
    let mut format = None;
    let mut data = None;
    let mut at = 12;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let size =
            u32::from_le_bytes(bytes[at + 4..at + 8].try_into().expect("four bytes")) as usize;
        let start = at + 8;
        // A data chunk whose declared size runs past the end keeps the bytes present.
        let end = start.saturating_add(size).min(bytes.len());
        let body = &bytes[start..end];
        match id {
            b"fmt " => {
                if body.len() < 16 {
                    return Err(Refusal::Truncated);
                }
                let word = |offset: usize| u16::from_le_bytes([body[offset], body[offset + 1]]);
                let mut tag = word(0);
                if tag == FORMAT_EXTENSIBLE {
                    if body.len() < 26 {
                        return Err(Refusal::Truncated);
                    }
                    tag = word(24);
                }
                format = Some(FormatChunk {
                    tag,
                    channels: word(2),
                    rate: u32::from_le_bytes(body[4..8].try_into().expect("four bytes")),
                    block: word(12),
                    bits: word(14),
                });
            }
            b"data" => data = Some(body),
            _ => {}
        }
        at = start.saturating_add(size).saturating_add(size & 1);
    }
    let format = format.ok_or(Refusal::MissingFormat)?;
    let data = data.ok_or(Refusal::MissingData)?;
    if !matches!(format.channels, 1 | 2) {
        return Err(Refusal::Channels(format.channels));
    }
    if format.rate == 0 {
        return Err(Refusal::Rate(format.rate));
    }
    if matches!(format.tag, FORMAT_MS_ADPCM | FORMAT_IMA_ADPCM) {
        return crate::ogg::decode_wave(bytes.to_vec()).map_err(Refusal::Adpcm);
    }
    let width = match (format.tag, format.bits) {
        (FORMAT_PCM, 8 | 16 | 24 | 32) | (FORMAT_FLOAT, 32) | (FORMAT_ALAW | FORMAT_MULAW, 8) => {
            usize::from(format.bits / 8)
        }
        (tag, bits) => return Err(Refusal::Encoding { tag, bits }),
    };
    let sample_format = match (format.tag, width) {
        (FORMAT_PCM, 1) => Format::U8,
        (FORMAT_PCM, 3 | 4) => Format::S32,
        (FORMAT_FLOAT, _) => Format::F32,
        _ => Format::S16,
    };
    let channels = usize::from(format.channels);
    let block = usize::from(format.block).max(width * channels);
    let frames = data.len() / block;
    let mut samples = Vec::with_capacity(frames * channels);
    for frame in data.chunks_exact(block) {
        for channel in 0..channels {
            let sample = &frame[channel * width..(channel + 1) * width];
            samples.push(match (format.tag, width) {
                (FORMAT_PCM, 1) => f32::from(sample[0]) * DIVBY128 - 1.0,
                (FORMAT_PCM, 2) => {
                    f32::from(i16::from_le_bytes([sample[0], sample[1]])) * DIVBY32768
                }
                // The loader widens 24 bits to 32; the converter drops the low byte.
                (FORMAT_PCM, 3) => {
                    let value = i32::from_le_bytes([0, sample[0], sample[1], sample[2]]) >> 8;
                    value as f32 * DIVBY8388607
                }
                (FORMAT_PCM, 4) => {
                    let value = i32::from_le_bytes(sample.try_into().expect("four bytes")) >> 8;
                    value as f32 * DIVBY8388607
                }
                (FORMAT_ALAW, 1) => f32::from(alaw(sample[0])) * DIVBY32768,
                (FORMAT_MULAW, 1) => f32::from(mulaw(sample[0])) * DIVBY32768,
                (FORMAT_FLOAT, 4) => f32::from_le_bytes(sample.try_into().expect("four bytes")),
                _ => unreachable!("widths are checked above"),
            });
        }
    }
    Ok(Pcm {
        rate: format.rate,
        channels: format.channels,
        format: sample_format,
        samples,
    })
}

/// The 16-bit value of a G.711 A-law byte.
fn alaw(byte: u8) -> i16 {
    let byte = byte ^ 0x55;
    let segment = (byte >> 4) & 7;
    let mut value = (i16::from(byte & 0x0f) << 4) + 8;
    if segment > 0 {
        value = (value + 0x100) << (segment - 1);
    }
    if byte & 0x80 != 0 { value } else { -value }
}

/// The 16-bit value of a G.711 mu-law byte.
fn mulaw(byte: u8) -> i16 {
    const BIAS: i16 = 0x84;
    let byte = !byte;
    let value = ((i16::from(byte & 0x0f) << 3) + BIAS) << ((byte >> 4) & 7);
    if byte & 0x80 != 0 {
        BIAS - value
    } else {
        value - BIAS
    }
}

/// A synthetic PCM file for tests in this and consumer crates.
pub fn encode_pcm16(rate: u32, channels: u16, samples: &[i16]) -> Vec<u8> {
    let data = samples.len() * 2;
    let mut out = Vec::with_capacity(44 + data);
    out.extend_from_slice(b"RIFF");
    out.extend_from_slice(&(36 + data as u32).to_le_bytes());
    out.extend_from_slice(b"WAVEfmt ");
    out.extend_from_slice(&16u32.to_le_bytes());
    out.extend_from_slice(&FORMAT_PCM.to_le_bytes());
    out.extend_from_slice(&channels.to_le_bytes());
    out.extend_from_slice(&rate.to_le_bytes());
    out.extend_from_slice(&(rate * u32::from(channels) * 2).to_le_bytes());
    out.extend_from_slice(&(channels * 2).to_le_bytes());
    out.extend_from_slice(&16u16.to_le_bytes());
    out.extend_from_slice(b"data");
    out.extend_from_slice(&(data as u32).to_le_bytes());
    for sample in samples {
        out.extend_from_slice(&sample.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sixteen_bit_stereo_decodes_to_unit_range_frames() {
        let bytes = encode_pcm16(44_100, 2, &[0, 16_384, -32_768, 32_767]);
        let pcm = decode(&bytes).unwrap();
        assert_eq!((pcm.rate, pcm.channels, pcm.frames()), (44_100, 2, 2));
        assert_eq!(pcm.frame(0), [0.0, 0.5]);
        assert_eq!(pcm.frame(1)[0], -1.0);
        assert!((pcm.frame(1)[1] - 32_767.0 / 32_768.0).abs() < 1e-7);
    }

    #[test]
    fn mono_plays_on_both_sides_and_odd_chunks_are_padded() {
        let mut bytes = encode_pcm16(48_000, 1, &[8_192]);
        // Insert an odd-sized chunk before the data chunk: one pad byte follows it.
        let data = bytes.split_off(36);
        bytes.extend_from_slice(b"LIST");
        bytes.extend_from_slice(&3u32.to_le_bytes());
        bytes.extend_from_slice(&[1, 2, 3, 0]);
        bytes.extend_from_slice(&data);
        let pcm = decode(&bytes).unwrap();
        assert_eq!(pcm.frames(), 1);
        assert_eq!(pcm.frame(0), [0.25, 0.25]);
    }

    #[test]
    fn twenty_four_bit_samples_keep_their_sign() {
        let mut bytes = encode_pcm16(44_100, 1, &[]);
        // Rewrite the format as 24-bit mono with one frame of -0.5.
        bytes[32..34].copy_from_slice(&3u16.to_le_bytes());
        bytes[34..36].copy_from_slice(&24u16.to_le_bytes());
        bytes[40..44].copy_from_slice(&3u32.to_le_bytes());
        bytes.extend_from_slice(&[0x00, 0x00, 0xc0]);
        let pcm = decode(&bytes).unwrap();
        assert_eq!(pcm.format, Format::S32);
        // SDL scales the 24-bit value by 1/8388607, not 1/8388608.
        assert_eq!(pcm.samples, vec![-4_194_304.0 * DIVBY8388607]);
        assert_ne!(pcm.samples, vec![-0.5]);
    }

    #[test]
    fn eight_bit_samples_are_unsigned_and_double_floats_are_refused() {
        let mut bytes = encode_pcm16(44_100, 1, &[]);
        bytes[32..34].copy_from_slice(&1u16.to_le_bytes());
        bytes[34..36].copy_from_slice(&8u16.to_le_bytes());
        bytes[40..44].copy_from_slice(&2u32.to_le_bytes());
        bytes.extend_from_slice(&[0, 192]);
        let pcm = decode(&bytes).unwrap();
        assert_eq!(
            (pcm.format, pcm.samples.clone()),
            (Format::U8, vec![-1.0, 0.5])
        );
        // SDL's loader takes IEEE float at 32 bits only.
        let mut bytes = encode_pcm16(44_100, 1, &[0, 0, 0, 0]);
        bytes[20..22].copy_from_slice(&FORMAT_FLOAT.to_le_bytes());
        bytes[32..34].copy_from_slice(&8u16.to_le_bytes());
        bytes[34..36].copy_from_slice(&64u16.to_le_bytes());
        assert_eq!(
            decode(&bytes),
            Err(Refusal::Encoding {
                tag: FORMAT_FLOAT,
                bits: 64
            })
        );
    }

    #[test]
    fn unknown_encodings_are_refused_by_name() {
        let mut bytes = encode_pcm16(44_100, 1, &[0]);
        bytes[20..22].copy_from_slice(&0x55u16.to_le_bytes());
        bytes[34..36].copy_from_slice(&0u16.to_le_bytes());
        assert_eq!(
            decode(&bytes),
            Err(Refusal::Encoding { tag: 0x55, bits: 0 })
        );
        assert_eq!(
            decode(&bytes).unwrap_err().to_string(),
            "unsupported WAVE encoding 0x0055 at 0 bits"
        );
        assert_eq!(decode(b"OggS"), Err(Refusal::NotWave));
    }

    #[test]
    fn g711_bytes_decode_to_their_linear_values() {
        let mut bytes = encode_pcm16(8_000, 1, &[]);
        bytes[20..22].copy_from_slice(&FORMAT_ALAW.to_le_bytes());
        bytes[32..34].copy_from_slice(&1u16.to_le_bytes());
        bytes[34..36].copy_from_slice(&8u16.to_le_bytes());
        bytes[40..44].copy_from_slice(&4u32.to_le_bytes());
        bytes.extend_from_slice(&[0xd5, 0x55, 0xaa, 0x2a]);
        let pcm = decode(&bytes).unwrap();
        let values: Vec<f32> = pcm.samples.iter().map(|value| value * 32768.0).collect();
        assert_eq!(values, [8.0, -8.0, 32256.0, -32256.0]);
        bytes[20..22].copy_from_slice(&FORMAT_MULAW.to_le_bytes());
        let tail = bytes.len() - 4;
        bytes[tail..].copy_from_slice(&[0xff, 0x7f, 0x80, 0x00]);
        let pcm = decode(&bytes).unwrap();
        let values: Vec<f32> = pcm.samples.iter().map(|value| value * 32768.0).collect();
        assert_eq!(values, [0.0, 0.0, 32124.0, -32124.0]);
    }

    /// One mono IMA ADPCM block: a zero predictor at step index 0, then
    /// nibbles of +7 steps, so the samples rise from zero.
    fn ima_adpcm() -> (Vec<u8>, u16) {
        let block_align: u16 = 36;
        let per_block: u16 = (block_align - 4) * 2 + 1;
        let mut data = vec![0u8, 0, 0, 0];
        data.extend(std::iter::repeat_n(0x77u8, usize::from(block_align) - 4));
        let mut out = Vec::new();
        out.extend_from_slice(b"RIFF");
        out.extend_from_slice(&(4 + 28 + 12 + 8 + data.len() as u32).to_le_bytes());
        out.extend_from_slice(b"WAVEfmt ");
        out.extend_from_slice(&20u32.to_le_bytes());
        out.extend_from_slice(&FORMAT_IMA_ADPCM.to_le_bytes());
        out.extend_from_slice(&1u16.to_le_bytes());
        out.extend_from_slice(&8_000u32.to_le_bytes());
        out.extend_from_slice(
            &(8_000u32 * u32::from(block_align) / u32::from(per_block)).to_le_bytes(),
        );
        out.extend_from_slice(&block_align.to_le_bytes());
        out.extend_from_slice(&4u16.to_le_bytes());
        out.extend_from_slice(&2u16.to_le_bytes());
        out.extend_from_slice(&per_block.to_le_bytes());
        out.extend_from_slice(b"fact");
        out.extend_from_slice(&4u32.to_le_bytes());
        out.extend_from_slice(&u32::from(per_block).to_le_bytes());
        out.extend_from_slice(b"data");
        out.extend_from_slice(&(data.len() as u32).to_le_bytes());
        out.extend_from_slice(&data);
        (out, per_block)
    }

    #[test]
    fn ima_adpcm_decodes_as_the_game_loads_it() {
        let (out, per_block) = ima_adpcm();
        let pcm = decode(&out).unwrap();
        assert_eq!((pcm.rate, pcm.channels), (8_000, 1));
        assert_eq!(pcm.frames(), usize::from(per_block));
        assert_eq!(pcm.samples[0], 0.0);
        assert!(pcm.samples.windows(2).all(|pair| pair[1] >= pair[0]));
        assert!(*pcm.samples.last().unwrap() > 0.0);
    }

    /// Truncated and corrupted files are refused, never a panic, whether
    /// this decoder or the ADPCM one reads them.
    #[test]
    fn truncated_or_garbage_files_are_refused_without_panicking() {
        let (adpcm, _) = ima_adpcm();
        let pcm = encode_pcm16(44_100, 2, &[1, -2, 3, -4, 5, -6]);
        for bytes in [adpcm, pcm] {
            for end in 0..bytes.len() {
                let _ = decode(&bytes[..end]);
            }
            for at in 0..bytes.len() {
                for value in [0x00, 0xff, 0x7f, 0x80] {
                    let mut corrupt = bytes.clone();
                    corrupt[at] = value;
                    let _ = decode(&corrupt);
                }
            }
        }
        assert_eq!(decode(b"RIFF"), Err(Refusal::NotWave));
    }
}
