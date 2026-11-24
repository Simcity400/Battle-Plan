
use std::{fmt, ops::Range};

const FORMAT_PCM: u16 = 1;
const FORMAT_FLOAT: u16 = 3;
const FORMAT_ALAW: u16 = 6;
const FORMAT_MULAW: u16 = 7;
const FORMAT_EXTENSIBLE: u16 = 0xfffe;
const FORMAT_MS_ADPCM: u16 = 2;
const FORMAT_IMA_ADPCM: u16 = 0x11;
const SUBTYPE_TAGS: [u16; 6] = [
    FORMAT_PCM,
    FORMAT_MS_ADPCM,
    FORMAT_FLOAT,
    FORMAT_ALAW,
    FORMAT_MULAW,
    FORMAT_IMA_ADPCM,
];
const SUBTYPE_TAIL: [u8; 14] = [0, 0, 0, 0, 0x10, 0, 0x80, 0, 0, 0xaa, 0, 0x38, 0x9b, 0x71];

pub const DIVBY32768: f32 = 1.0 / 32_768.0;
const DIVBY128: f32 = 1.0 / 128.0;
const DIVBY8388607: f32 = 0.000_000_119_209_3;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    U8,
    S16,
    S32,
    F32,
}

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

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Refusal {
    NotWave,
    Truncated,
    MissingFormat,
    MissingData,
    FormatAfterData,
    ExtensibleHeader,
    Subtype([u8; 16]),
    Encoding {
        tag: u16,
        bits: u16,
    },
    BlockAlign {
        block: u16,
        channels: u16,
        bits: u16,
    },
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
            Self::FormatAfterData => write!(f, "WAVE format chunk after its data chunk"),
            Self::ExtensibleHeader => write!(f, "extensible WAVE format chunk too short"),
            Self::Subtype(guid) => {
                let word = |at: usize| u16::from_le_bytes([guid[at], guid[at + 1]]);
                write!(
                    f,
                    "unsupported WAVE extensible subtype {:08x}-{:04x}-{:04x}-",
                    u32::from_le_bytes([guid[0], guid[1], guid[2], guid[3]]),
                    word(4),
                    word(6)
                )?;
                for (index, byte) in guid[8..].iter().enumerate() {
                    if index == 2 {
                        write!(f, "-")?;
                    }
                    write!(f, "{byte:02x}")?;
                }
                Ok(())
            }
            Self::Encoding { tag, bits } => {
                write!(f, "unsupported WAVE encoding {tag:#06x} at {bits} bits")
            }
            Self::BlockAlign {
                block,
                channels,
                bits,
            } => write!(
                f,
                "unsupported WAVE block alignment {block} for {channels} channels at {bits} bits"
            ),
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

impl FormatChunk {
    fn read(body: &[u8]) -> Result<Self, Refusal> {
        if body.len() < 16 {
            return Err(Refusal::Truncated);
        }
        let word = |offset: usize| u16::from_le_bytes([body[offset], body[offset + 1]]);
        let mut tag = word(0);
        if tag == FORMAT_EXTENSIBLE {
            if body.len() < 40 || word(16) < 22 {
                return Err(Refusal::ExtensibleHeader);
            }
            let subtype: [u8; 16] = body[24..40].try_into().expect("sixteen bytes");
            tag = SUBTYPE_TAGS
                .into_iter()
                .find(|plain| subtype[..2] == plain.to_le_bytes() && subtype[2..] == SUBTYPE_TAIL)
                .ok_or(Refusal::Subtype(subtype))?;
        }
        Ok(Self {
            tag,
            channels: word(2),
            rate: u32::from_le_bytes(body[4..8].try_into().expect("four bytes")),
            block: word(12),
            bits: word(14),
        })
    }
}

pub fn decode(bytes: &[u8]) -> Result<Pcm, Refusal> {
    if bytes.len() < 12 || &bytes[..4] != b"RIFF" || &bytes[8..12] != b"WAVE" {
        return Err(Refusal::NotWave);
    }
    let mut format = None;
    let mut data = None;
    let mut skipped = Vec::new();
    let mut at = 12;
    while at + 8 <= bytes.len() {
        let id = &bytes[at..at + 4];
        let size =
            u32::from_le_bytes(bytes[at + 4..at + 8].try_into().expect("four bytes")) as usize;
        let start = at + 8;
        let end = start.saturating_add(size).min(bytes.len());
        let body = &bytes[start..end];
        let next = start.saturating_add(size).saturating_add(size & 1);
        match id {
            b"fmt " if format.is_some() => skipped.push(at..next.min(bytes.len())),
            b"fmt " if data.is_some() => return Err(Refusal::FormatAfterData),
            b"fmt " => format = Some(body),
            b"data" if data.is_none() => {
                data = Some(body);
                if format.is_some() {
                    break;
                }
            }
            _ => {}
        }
        at = next;
    }
    let format = format.ok_or(Refusal::MissingFormat)?;
    let data = data.ok_or(Refusal::MissingData)?;
    let format = FormatChunk::read(format)?;
    if !matches!(format.channels, 1 | 2) {
        return Err(Refusal::Channels(format.channels));
    }
    if format.rate == 0 {
        return Err(Refusal::Rate(format.rate));
    }
    if matches!(format.tag, FORMAT_MS_ADPCM | FORMAT_IMA_ADPCM) {
        return crate::ogg::decode_wave(without(bytes, &skipped)).map_err(Refusal::Adpcm);
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
    let frame_bytes = width * channels;
    let block = usize::from(format.block);
    let aligned = match format.tag {
        FORMAT_ALAW | FORMAT_MULAW => block == channels,
        _ => block != 0 && frame_bytes.is_multiple_of(block),
    };
    if !aligned {
        return Err(Refusal::BlockAlign {
            block: format.block,
            channels: format.channels,
            bits: format.bits,
        });
    }
    let frames = data.len() / frame_bytes;
    let mut samples = Vec::with_capacity(frames * channels);
    for frame in data.chunks_exact(frame_bytes) {
        for channel in 0..channels {
            let sample = &frame[channel * width..(channel + 1) * width];
            samples.push(match (format.tag, width) {
                (FORMAT_PCM, 1) => f32::from(sample[0]) * DIVBY128 - 1.0,
                (FORMAT_PCM, 2) => {
                    f32::from(i16::from_le_bytes([sample[0], sample[1]])) * DIVBY32768
                }
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

fn without(bytes: &[u8], chunks: &[Range<usize>]) -> Vec<u8> {
    let mut kept = Vec::with_capacity(bytes.len());
    let mut from = 0;
    for chunk in chunks {
        kept.extend_from_slice(&bytes[from..chunk.start]);
        from = chunk.end;
    }
    kept.extend_from_slice(&bytes[from..]);
    let size = u32::from_le_bytes(kept[4..8].try_into().expect("four bytes"));
    if !chunks.is_empty() && size != u32::MAX {
        let removed = u32::try_from(bytes.len() - kept.len()).unwrap_or(u32::MAX);
        kept[4..8].copy_from_slice(&size.saturating_sub(removed).to_le_bytes());
    }
    kept
}

fn alaw(byte: u8) -> i16 {
    let byte = byte ^ 0x55;
    let segment = (byte >> 4) & 7;
    let mut value = (i16::from(byte & 0x0f) << 4) + 8;
    if segment > 0 {
        value = (value + 0x100) << (segment - 1);
    }
    if byte & 0x80 != 0 { value } else { -value }
}

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
        bytes[32..34].copy_from_slice(&3u16.to_le_bytes());
        bytes[34..36].copy_from_slice(&24u16.to_le_bytes());
        bytes[40..44].copy_from_slice(&3u32.to_le_bytes());
        bytes.extend_from_slice(&[0x00, 0x00, 0xc0]);
        let pcm = decode(&bytes).unwrap();
        assert_eq!(pcm.format, Format::S32);
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
            (pcm.format, pcm.samples),
            (Format::U8, vec![-1.0, 0.5])
        );
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
    fn later_chunks_never_replace_the_first_format_and_data() {
        let mut bytes = encode_pcm16(8_000, 1, &[8_192]);
        bytes.extend_from_slice(b"data");
        bytes.extend_from_slice(&2u32.to_le_bytes());
        bytes.extend_from_slice(&(-8_192i16).to_le_bytes());
        bytes.extend_from_slice(b"fmt ");
        bytes.extend_from_slice(&4u32.to_le_bytes());
        bytes.extend_from_slice(&[0; 4]);
        assert_eq!(decode(&bytes).unwrap().samples, [0.25]);
        let mut bytes = encode_pcm16(8_000, 1, &[8_192, 8_192]);
        let data = bytes.split_off(36);
        bytes.extend_from_slice(&encode_pcm16(8_000, 2, &[])[12..36]);
        bytes.extend_from_slice(&data);
        let pcm = decode(&bytes).unwrap();
        assert_eq!((pcm.channels, pcm.samples), (1, vec![0.25, 0.25]));
        let pcm = encode_pcm16(8_000, 1, &[8_192]);
        let mut bytes = pcm[..12].to_vec();
        bytes.extend_from_slice(&pcm[36..]);
        bytes.extend_from_slice(&pcm[12..36]);
        assert_eq!(decode(&bytes), Err(Refusal::FormatAfterData));
    }

    fn extensible(extension: u16, subtype: [u8; 16], length: usize) -> Vec<u8> {
        let mut body = Vec::new();
        body.extend_from_slice(&FORMAT_EXTENSIBLE.to_le_bytes());
        body.extend_from_slice(&1u16.to_le_bytes());
        body.extend_from_slice(&8_000u32.to_le_bytes());
        body.extend_from_slice(&16_000u32.to_le_bytes());
        body.extend_from_slice(&2u16.to_le_bytes());
        body.extend_from_slice(&16u16.to_le_bytes());
        body.extend_from_slice(&extension.to_le_bytes());
        body.extend_from_slice(&16u16.to_le_bytes());
        body.extend_from_slice(&4u32.to_le_bytes());
        body.extend_from_slice(&subtype);
        body.truncate(length);
        let mut out = b"RIFF\0\0\0\0WAVEfmt ".to_vec();
        out.extend_from_slice(&(body.len() as u32).to_le_bytes());
        out.extend_from_slice(&body);
        out.extend_from_slice(b"data");
        out.extend_from_slice(&2u32.to_le_bytes());
        out.extend_from_slice(&8_192i16.to_le_bytes());
        out
    }

    #[test]
    fn extensible_formats_need_their_whole_extension_and_a_known_subtype() {
        let mut pcm = [0u8; 16];
        pcm[0] = 1;
        pcm[2..].copy_from_slice(&SUBTYPE_TAIL);
        let decoded = decode(&extensible(22, pcm, 40)).unwrap();
        assert_eq!((decoded.format, decoded.samples), (Format::S16, vec![0.25]));
        assert_eq!(
            decode(&extensible(22, pcm, 26)),
            Err(Refusal::ExtensibleHeader)
        );
        assert_eq!(
            decode(&extensible(0, pcm, 40)),
            Err(Refusal::ExtensibleHeader)
        );
        let mut unknown = pcm;
        unknown[15] ^= 1;
        let refusal = decode(&extensible(22, unknown, 40)).unwrap_err();
        assert_eq!(refusal, Refusal::Subtype(unknown));
        assert_eq!(
            refusal.to_string(),
            "unsupported WAVE extensible subtype 00000001-0000-0010-8000-00aa00389b70"
        );
    }

    #[test]
    fn block_alignments_the_loader_refuses_are_refused() {
        let mut bytes = encode_pcm16(8_000, 1, &[8_192, 0]);
        for block in [0u16, 4] {
            bytes[32..34].copy_from_slice(&block.to_le_bytes());
            assert_eq!(
                decode(&bytes),
                Err(Refusal::BlockAlign {
                    block,
                    channels: 1,
                    bits: 16
                })
            );
        }
        let mut bytes = encode_pcm16(8_000, 2, &[8_192, -8_192]);
        bytes[32..34].copy_from_slice(&2u16.to_le_bytes());
        assert_eq!(decode(&bytes).unwrap().samples, [0.25, -0.25]);
        let mut bytes = encode_pcm16(8_000, 1, &[]);
        bytes[20..22].copy_from_slice(&FORMAT_ALAW.to_le_bytes());
        bytes[32..34].copy_from_slice(&2u16.to_le_bytes());
        bytes[34..36].copy_from_slice(&8u16.to_le_bytes());
        bytes[40..44].copy_from_slice(&2u32.to_le_bytes());
        bytes.extend_from_slice(&[0xd5, 0x55]);
        assert_eq!(
            decode(&bytes),
            Err(Refusal::BlockAlign {
                block: 2,
                channels: 1,
                bits: 8
            })
        );
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

    #[test]
    #[expect(clippy::single_range_in_vec_init, reason = "without takes the list of byte ranges to cut, and this test cuts one")]
    fn adpcm_reads_only_the_first_format_chunk() {
        let (original, _) = ima_adpcm();
        let second = &encode_pcm16(8_000, 2, &[])[12..36];
        let mut doubled = original[..40].to_vec();
        doubled.extend_from_slice(second);
        doubled.extend_from_slice(&original[40..]);
        let size = u32::from_le_bytes(original[4..8].try_into().unwrap()) + second.len() as u32;
        doubled[4..8].copy_from_slice(&size.to_le_bytes());
        assert_eq!(without(&doubled, &[40..64]), original);
        assert_eq!(decode(&doubled), decode(&original));
        assert!(decode(&doubled).is_ok());
    }

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
