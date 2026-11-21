use super::*;

fn pcm(rate: u32, channels: u16, format: Format, samples: Vec<f32>) -> Pcm {
    Pcm {
        rate,
        channels,
        format,
        samples,
    }
}

#[test]
fn sixteen_bit_stereo_at_the_mixer_rate_loads_unchanged() {
    let values = [0i16, 1, -1, 32_767, -32_768, 12_345];
    let samples = values
        .iter()
        .map(|value| f32::from(*value) * DIVBY32768)
        .collect();
    assert_eq!(to_mixer(&pcm(44_100, 2, Format::S16, samples)), values);
}

#[test]
fn other_files_convert_through_float_as_sdl_does() {
    let half = 16_384.0 * DIVBY32768;
    let converted = to_mixer(&pcm(44_100, 1, Format::S16, vec![half; 10]));
    assert_eq!(converted.len(), 20);
    assert!(converted[..16].iter().all(|value| *value == 16_384));
    assert!(converted[16..].iter().all(|value| *value == 16_383));
    let edges = to_mixer(&pcm(44_100, 2, Format::F32, vec![2.0, -2.0]));
    assert_eq!(edges, [32_767, -32_768]);
    let block = to_mixer(&pcm(44_100, 2, Format::F32, vec![-1.0; 8]));
    assert_eq!(block, [-32_767; 8]);
}

#[test]
fn the_buffer_offset_decides_which_samples_take_the_sse2_path() {
    let samples = vec![0.5f32; 12];
    let aligned = s16_sse2(&samples, 0);
    assert_eq!(&aligned[..8], &[16_384; 8]);
    assert_eq!(&aligned[8..], &[16_383; 4]);
    assert_eq!(s16_sse2(&samples, 8), vec![16_383; 12]);
}

#[test]
fn the_resampler_filter_is_a_windowed_sinc() {
    let filter = filter();
    assert_eq!(filter.table.len(), FILTER_SIZE);
    assert_eq!(filter.table[0], 1.0);
    for crossing in 1..ZERO_CROSSINGS {
        assert!(filter.table[crossing * SAMPLES_PER_ZERO_CROSSING].abs() < 1e-6);
    }
    assert!(filter.table[256] > 0.6 && filter.table[256] < 0.65);
    assert_eq!(filter.differences[FILTER_SIZE - 1], 0.0);
    assert_eq!(filter.differences[0], filter.table[1] - filter.table[0]);
}

#[test]
fn other_rates_resample_bandlimited_with_silence_past_both_ends() {
    let samples = vec![0.5f32; 4_800 * 2];
    let converted = to_mixer(&pcm(48_000, 2, Format::F32, samples));
    assert_eq!(converted.len(), 4_409 * 2);
    let left: Vec<i16> = converted.iter().step_by(2).copied().collect();
    assert!(
        left[8..4_400]
            .iter()
            .all(|value| (16_382..=16_386).contains(value))
    );
    assert!(left[1] < 16_200 && left[2] > 16_450, "{:?}", &left[..8]);
    assert!(left[4_408] < 16_000, "{:?}", &left[4_400..]);
}

#[test]
fn a_device_in_the_mixer_format_takes_the_mix_unchanged() {
    let mut stream = Stream::new(44_100, 2, DeviceFormat::S16).unwrap();
    let mix: Vec<i16> = (0..MIXER_FRAMES as i16 * 2).collect();
    stream.put(&mix);
    let out: Vec<i16> = stream
        .queue()
        .drain(..)
        .map(|value| (value * 32_768.0) as i16)
        .collect();
    assert_eq!(out, mix);
}

#[test]
fn a_device_at_another_rate_takes_whole_frames_of_each_put() {
    let mut stream = Stream::new(48_000, 2, DeviceFormat::F32).unwrap();
    let mix = vec![8_192i16; MIXER_FRAMES * 2];
    stream.put(&mix);
    assert_eq!(stream.queue().len(), 557 * 2);
    stream.put(&mix);
    assert_eq!(stream.queue().len(), (557 + 1_114) * 2);
    let level = stream.queue()[1_400];
    assert!((level - 0.25).abs() < 0.01, "{level}");
}

#[test]
fn surround_devices_up_mix_as_sdl_does() {
    let mut stream = Stream::new(44_100, 6, DeviceFormat::F32).unwrap();
    let mut mix = Vec::new();
    for _ in 0..MIXER_FRAMES {
        mix.extend_from_slice(&[16_384i16, 0]);
    }
    stream.put(&mix);
    let frame: Vec<f32> = stream.queue().iter().take(6).copied().collect();
    let (lf, rf) = (0.5f32, 0.0f32);
    let ce = (lf + rf) * 0.5;
    assert_eq!(
        frame,
        [
            0.571 * (lf + (lf - 0.5 * ce)),
            0.571 * (rf + (rf - 0.5 * ce)),
            ce,
            0.0,
            lf,
            rf
        ]
    );
    assert!(Stream::new(44_100, 3, DeviceFormat::F32).is_none());
    let mut mono = Stream::new(44_100, 1, DeviceFormat::F32).unwrap();
    mono.put(&mix);
    assert_eq!(mono.queue()[0], 0.25);
}
