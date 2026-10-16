//! The audio conversions of SDL 2.0.20, which the game links and mixes
//! through: its sample loader converts every file to 16-bit stereo at the
//! mixer rate ([`to_mixer`]), and its output device, opened at the mixer's
//! 16-bit stereo 44,100 Hz with no format changes allowed, converts each
//! 1,024-frame mix to the device's own format through an audio stream
//! ([`Stream`]).
//!
//! Altered from SDL 2.0.20's `SDL_audiocvt.c` and `SDL_audiotypecvt.c`
//! (Copyright (C) 1997-2022 Sam Lantinga; zlib licence, see
//! `LICENSES/SDL-LICENSE.txt`): the same float and double arithmetic in the
//! same order, rewritten in Rust for the conversions the game reaches. The
//! 16-bit output conversion follows SDL's SSE2 path, which converts aligned
//! blocks of eight samples rounding to nearest and the rest truncating.

use installed_content_sound::wav::{DIVBY32768, Format, Pcm};
use std::{collections::VecDeque, sync::OnceLock};

/// The mixer's rate, which the loader converts to and the device stream
/// converts from.
pub const MIXER_RATE: i32 = 44_100;
/// The resampler's table: its zero crossings and samples per crossing.
const ZERO_CROSSINGS: usize = 5;
const SAMPLES_PER_ZERO_CROSSING: usize = 1 << (16 / 2 + 1);
const FILTER_SIZE: usize = SAMPLES_PER_ZERO_CROSSING * ZERO_CROSSINGS + 1;

struct Filter {
    table: Vec<f32>,
    differences: Vec<f32>,
}

/// SDL's modified Bessel function of the first kind.
fn bessel(x: f64) -> f64 {
    let half = x / 2.0;
    let mut i0 = 1.0f64;
    let mut f = 1.0f64;
    let mut i = 1i32;
    loop {
        let diff = half.powf(f64::from(i * 2)) / f.powf(2.0);
        if diff < f64::from(1.0e-21f32) {
            break;
        }
        i0 += diff;
        i += 1;
        f *= f64::from(i);
    }
    i0
}

/// The Kaiser-windowed sinc table and its differences, built once.
fn filter() -> &'static Filter {
    static FILTER: OnceLock<Filter> = OnceLock::new();
    FILTER.get_or_init(|| {
        let decibels = 80.0f64;
        let beta = 0.1102 * (decibels - 8.7);
        let length = FILTER_SIZE as i32;
        let last = length - 1;
        let half = last / 2;
        let mut table = vec![0.0f32; FILTER_SIZE];
        let mut differences = vec![0.0f32; FILTER_SIZE];
        table[0] = 1.0;
        for i in 1..length {
            let position = (f64::from(i - last) / 2.0) / f64::from(half);
            let kaiser = bessel(beta * (1.0 - position.powf(2.0)).sqrt()) / bessel(beta);
            table[(length - i) as usize] = kaiser as f32;
        }
        for i in 1..FILTER_SIZE {
            let x = (i as f32 / SAMPLES_PER_ZERO_CROSSING as f32) * std::f32::consts::PI;
            table[i] *= x.sin() / x;
            differences[i - 1] = table[i] - table[i - 1];
        }
        differences[FILTER_SIZE - 1] = 0.0;
        Filter { table, differences }
    })
}

/// The frames of padding the resampler reads on each side.
fn padding(in_rate: i32, out_rate: i32) -> usize {
    if in_rate == out_rate {
        0
    } else if in_rate > out_rate {
        ((SAMPLES_PER_ZERO_CROSSING as i32 * in_rate) as f32 / out_rate as f32).ceil() as usize
    } else {
        SAMPLES_PER_ZERO_CROSSING
    }
}

/// Resamples `input` (interleaved `channels`), reading `left` and `right`
/// padding past its ends, as SDL's bandlimited resampler does in one call;
/// its output position restarts at zero each call.
fn resample(
    channels: usize,
    in_rate: i32,
    out_rate: i32,
    left: &[f32],
    right: &[f32],
    input: &[f32],
) -> Vec<f32> {
    let filter = filter();
    let rate = f64::from(in_rate);
    let step = 1.0 / f64::from(out_rate as f32);
    let ratio = f64::from(out_rate as f32 / in_rate as f32);
    let padding = padding(in_rate, out_rate) as i64;
    let frames = input.len() / channels;
    let out_frames = (frames as f64 * ratio) as i64;
    let mut out = Vec::with_capacity(out_frames.max(0) as usize * channels);
    let mut time = 0.0f64;
    for _ in 0..out_frames {
        let source = (time * f64::from(in_rate)) as i64;
        let at = source as f64 / rate;
        let next = (source + 1) as f64 / rate;
        let first = 1.0 - ((next - time) / (next - at));
        let first_index = (first * SAMPLES_PER_ZERO_CROSSING as f64) as usize;
        let second = 1.0 - first;
        let second_index = (second * SAMPLES_PER_ZERO_CROSSING as f64) as usize;
        for channel in 0..channels {
            let mut sample = 0.0f32;
            let mut j = 0usize;
            while first_index + j * SAMPLES_PER_ZERO_CROSSING < FILTER_SIZE {
                let frame = source - j as i64;
                let value = if frame < 0 {
                    left[((padding + frame) as usize) * channels + channel]
                } else {
                    input[frame as usize * channels + channel]
                };
                let index = first_index + j * SAMPLES_PER_ZERO_CROSSING;
                sample += (f64::from(value)
                    * (f64::from(filter.table[index])
                        + first * f64::from(filter.differences[index])))
                    as f32;
                j += 1;
            }
            let mut j = 0usize;
            while second_index + j * SAMPLES_PER_ZERO_CROSSING < FILTER_SIZE {
                let frame = source + 1 + j as i64;
                let value = if frame >= frames as i64 {
                    right[(frame as usize - frames) * channels + channel]
                } else {
                    input[frame as usize * channels + channel]
                };
                let index = second_index + j * SAMPLES_PER_ZERO_CROSSING;
                sample += (f64::from(value)
                    * (f64::from(filter.table[index])
                        + second * f64::from(filter.differences[index])))
                    as f32;
                j += 1;
            }
            out.push(sample);
        }
        time += step;
    }
    out
}

/// SDL's scalar float-to-16-bit conversion of one sample.
fn s16_scalar(sample: f32) -> i16 {
    if sample >= 1.0 {
        32_767
    } else if sample <= -1.0 {
        -32_768
    } else {
        (sample * 32_767.0) as i16
    }
}

/// SDL's SSE2 float-to-16-bit conversion of a buffer converted in place.
/// `offset` is the buffer's start address modulo 16: samples before the
/// first 16-byte aligned output convert one at a time, then aligned
/// blocks of eight clamp and round to nearest, and the rest convert one at
/// a time again.
fn s16_sse2(samples: &[f32], offset: usize) -> Vec<i16> {
    let mut out = Vec::with_capacity(samples.len());
    let mut index = 0;
    let mut address = offset % 16;
    while index < samples.len() && !address.is_multiple_of(16) {
        out.push(s16_scalar(samples[index]));
        index += 1;
        address += 2;
    }
    // The input, four bytes a sample, must be aligned too.
    if (offset + index * 4).is_multiple_of(16) {
        while samples.len() - index >= 8 {
            for sample in &samples[index..index + 8] {
                let scaled = sample.clamp(-1.0, 1.0) * 32_767.0;
                out.push(scaled.round_ties_even() as i32 as i16);
            }
            index += 8;
        }
    }
    out.extend(samples[index..].iter().map(|sample| s16_scalar(*sample)));
    out
}

/// A loaded file converted to the mixer's 16-bit stereo at
/// [`MIXER_RATE`], as the game's loader converts it: nothing for 16-bit
/// stereo at that rate; otherwise to float, mono duplicated to both
/// sides, resampled with silence past both ends, then to 16 bits.
pub fn to_mixer(pcm: &Pcm) -> Vec<i16> {
    let rate = pcm.rate as i32;
    let channels = usize::from(pcm.channels.clamp(1, 2));
    if rate == MIXER_RATE && channels == 2 && pcm.format == Format::S16 {
        return pcm
            .samples
            .iter()
            .map(|sample| (sample * 32_768.0) as i16)
            .collect();
    }
    let mut stereo: Vec<f32> = if channels == 1 {
        pcm.samples
            .iter()
            .flat_map(|sample| [*sample, *sample])
            .collect()
    } else {
        pcm.samples.clone()
    };
    if rate != MIXER_RATE {
        let silence = vec![0.0f32; padding(rate, MIXER_RATE) * 2];
        stereo = resample(2, rate, MIXER_RATE, &silence, &silence, &stereo);
    }
    // The converted buffer is a fresh, 16-byte aligned allocation.
    s16_sse2(&stereo, 0)
}

/// The device's sample format.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DeviceFormat {
    F32,
    S16,
}

/// The mixer's channels up-mixed to `channels` as SDL's converters do;
/// stereo stays as it is. `None` for counts SDL does not convert to.
fn up_mix(stereo: &[f32], channels: usize) -> Option<Vec<f32>> {
    let frames = stereo.chunks_exact(2);
    Some(match channels {
        2 => stereo.to_vec(),
        4 => frames
            .flat_map(|frame| [frame[0], frame[1], frame[0], frame[1]])
            .collect(),
        6 | 8 => {
            let mut out = Vec::with_capacity(stereo.len() / 2 * channels);
            for frame in frames {
                let (lf, rf) = (frame[0], frame[1]);
                let ce = (lf + rf) * 0.5;
                let surround = [
                    0.571 * (lf + (lf - 0.5 * ce)),
                    0.571 * (rf + (rf - 0.5 * ce)),
                    ce,
                    0.0,
                    lf,
                    rf,
                ];
                if channels == 6 {
                    out.extend_from_slice(&surround);
                    continue;
                }
                let [mut lf, mut rf, ce, lfe, mut lb, mut rb] = surround;
                let ls = (lf + lb) * 0.5;
                let rs = (rf + rb) * 0.5;
                lf += lf - ls;
                rf += rf - rs;
                lb += lb - ls;
                rb += rb - rs;
                out.extend_from_slice(&[0.5 * lf, 0.5 * rf, ce, lfe, 0.5 * lb, 0.5 * rb, ls, rs]);
            }
            out
        }
        _ => return None,
    })
}

/// The device side of SDL's audio stream: each [`MIXER_FRAMES`]-frame mix
/// (16-bit stereo at [`MIXER_RATE`]) is put in, converted to float,
/// resampled with the previous put's tail as left padding and the held
/// tail of this one as right padding, up-mixed and converted to the
/// device's format, and queued for the device.
#[derive(Debug)]
pub struct Stream {
    rate: i32,
    channels: usize,
    format: DeviceFormat,
    /// Channels while resampling: mono devices mix down first.
    resampled: usize,
    padding: usize,
    first: bool,
    /// The left padding: the end of the last resampled input.
    state: Vec<f32>,
    /// The input held back as the next put's start and this one's right padding.
    held: Vec<f32>,
    /// Puts shorter than the padding wait here.
    staging: Vec<i16>,
    queue: VecDeque<f32>,
}

/// Frames the game's mixer renders per device callback.
pub const MIXER_FRAMES: usize = 1024;

impl Stream {
    /// `None` for channel counts SDL does not convert the mixer's stereo to.
    pub fn new(rate: u32, channels: usize, format: DeviceFormat) -> Option<Self> {
        if !matches!(channels, 1 | 2 | 4 | 6 | 8) || rate == 0 {
            return None;
        }
        let rate = rate as i32;
        let resampled = channels.min(2);
        let padding = padding(MIXER_RATE, rate);
        Some(Self {
            rate,
            channels,
            format,
            resampled,
            padding,
            first: true,
            state: vec![0.0; padding * resampled],
            held: Vec::new(),
            staging: Vec::new(),
            queue: VecDeque::new(),
        })
    }

    /// Converted samples waiting for the device, interleaved, in [-1, 1]
    /// for float devices and 16-bit steps scaled by 1/32768 otherwise.
    pub fn queue(&mut self) -> &mut VecDeque<f32> {
        &mut self.queue
    }

    /// Puts one mix (interleaved 16-bit stereo) into the stream.
    pub fn put(&mut self, mix: &[i16]) {
        let staging = self.padding * 2;
        let mut mix = mix;
        while !mix.is_empty() {
            if self.staging.is_empty() && mix.len() >= staging {
                let all = mix.to_vec();
                self.put_internal(&all);
                return;
            }
            if self.staging.len() + mix.len() < staging {
                self.staging.extend_from_slice(mix);
                return;
            }
            let take = staging - self.staging.len();
            self.staging.extend_from_slice(&mix[..take]);
            let staged = std::mem::take(&mut self.staging);
            self.put_internal(&staged);
            mix = &mix[take..];
        }
    }

    fn put_internal(&mut self, mix: &[i16]) {
        // A device in the mixer's own format opens without a stream.
        if self.rate == MIXER_RATE && self.channels == 2 && self.format == DeviceFormat::S16 {
            self.queue
                .extend(mix.iter().map(|sample| f32::from(*sample) * DIVBY32768));
            return;
        }
        let float: Vec<f32> = mix
            .iter()
            .map(|sample| f32::from(*sample) * DIVBY32768)
            .collect();
        let down = |float: Vec<f32>| -> Vec<f32> {
            float
                .chunks_exact(2)
                .map(|frame| (frame[0] + frame[1]) * 0.5)
                .collect()
        };
        let (converted, offset) = if self.rate == MIXER_RATE {
            let converted = if self.channels == 1 {
                down(float)
            } else {
                up_mix(&float, self.channels).expect("channel counts are checked on creation")
            };
            (converted, 0)
        } else {
            let chans = self.resampled;
            let pre = if chans == 1 { down(float) } else { float };
            let mut work = if self.first {
                Vec::new()
            } else {
                std::mem::take(&mut self.held)
            };
            self.first = false;
            work.extend_from_slice(&pre);
            // The resampled output follows the work buffer's input in it.
            let offset = (work.len() * 4) % 16;
            let needed = self.padding * chans;
            let tail = work.split_off(work.len() - needed.min(work.len()));
            let out = if work.is_empty() {
                Vec::new()
            } else {
                let out = resample(chans, MIXER_RATE, self.rate, &self.state, &tail, &work);
                // The left padding's end takes the end of what was resampled;
                // its start keeps what it held.
                let keep = needed.min(work.len());
                let start = self.state.len() - keep;
                self.state[start..].copy_from_slice(&work[work.len() - keep..]);
                out
            };
            self.held = tail;
            let converted = if self.channels == 1 {
                out
            } else {
                up_mix(&out, self.channels).expect("channel counts are checked on creation")
            };
            (converted, offset)
        };
        match self.format {
            DeviceFormat::F32 => self.queue.extend(converted),
            DeviceFormat::S16 => self.queue.extend(
                s16_sse2(&converted, offset)
                    .into_iter()
                    .map(|sample| f32::from(sample) * DIVBY32768),
            ),
        }
    }
}

#[cfg(test)]
mod tests;
