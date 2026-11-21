
use crate::wav::Pcm;
use symphonia::core::{
    codecs::audio::{AudioDecoder, AudioDecoderOptions},
    errors::Error,
    formats::{FormatOptions, FormatReader, TrackType, probe::Hint},
    io::MediaSourceStream,
    meta::MetadataOptions,
};

#[derive(Clone, Copy, Debug)]
enum Container {
    Ogg,
    Wave,
}

impl Container {
    fn name(self) -> &'static str {
        match self {
            Self::Ogg => "Ogg Vorbis",
            Self::Wave => "WAVE",
        }
    }

    fn refusal(self) -> &'static str {
        match self {
            Self::Ogg => "not an Ogg Vorbis stream",
            Self::Wave => "not a WAVE stream",
        }
    }
}

pub fn decode(bytes: Vec<u8>) -> Result<Pcm, String> {
    read(bytes, Container::Ogg, false)
}

pub(crate) fn decode_wave(bytes: Vec<u8>) -> Result<Pcm, String> {
    read(bytes, Container::Wave, false)
}

pub fn probe<B: AsRef<[u8]> + Send + Sync + std::panic::UnwindSafe + 'static>(bytes: B) -> Result<(), String> {
    read(bytes, Container::Ogg, true).map(drop)
}

pub fn open<B: AsRef<[u8]> + Send + Sync + std::panic::UnwindSafe + 'static>(bytes: B) -> Result<Stream, String> {
    guarded(Container::Ogg, || Stream::open(bytes, Container::Ogg))
}

#[derive(Clone, Debug, PartialEq)]
pub struct Packet {
    pub rate: u32,
    pub channels: u16,
    pub samples: Vec<f32>,
}

pub struct Stream {
    container: Container,
    format: Box<dyn FormatReader>,
    decoder: Box<dyn AudioDecoder>,
    track_id: u32,
    spec: Option<(u32, usize)>,
}

impl Stream {
    fn open<B: AsRef<[u8]> + Send + Sync + 'static>(bytes: B, container: Container) -> Result<Self, String> {
        let name = container.name();
        let (magic, extension): (&[u8], _) = match container {
            Container::Ogg => (b"OggS", "ogg"),
            Container::Wave => (b"RIFF", "wav"),
        };
        if !bytes.as_ref().starts_with(magic) {
            return Err(container.refusal().into());
        }
        let source =
            MediaSourceStream::new(Box::new(std::io::Cursor::new(bytes)), Default::default());
        let mut hint = Hint::new();
        hint.with_extension(extension);
        let format = symphonia::default::get_probe()
            .probe(
                &hint,
                source,
                FormatOptions::default(),
                MetadataOptions::default(),
            )
            .map_err(|error| format!("{} Symphonia reads ({error})", container.refusal()))?;
        let track = format
            .default_track(TrackType::Audio)
            .ok_or_else(|| format!("{name} stream without an audio track"))?;
        let track_id = track.id;
        let parameters = track
            .codec_params
            .as_ref()
            .and_then(|parameters| parameters.audio())
            .ok_or_else(|| format!("{name} track without audio parameters"))?
            .clone();
        let decoder = symphonia::default::get_codecs()
            .make_audio_decoder(&parameters, &AudioDecoderOptions::default())
            .map_err(|error| format!("unsupported {name} codec ({error})"))?;
        Ok(Self {
            container,
            format,
            decoder,
            track_id,
            spec: None,
        })
    }

    pub fn decode_next(&mut self) -> Result<Option<Packet>, String> {
        let container = self.container;
        guarded(
            container,
            std::panic::AssertUnwindSafe(|| self.next_packet()),
        )
    }

    fn next_packet(&mut self) -> Result<Option<Packet>, String> {
        let name = self.container.name();
        loop {
            let packet = match self.format.next_packet() {
                Ok(Some(packet)) => packet,
                Ok(None) => return Ok(None),
                Err(Error::IoError(error)) if error.kind() == std::io::ErrorKind::UnexpectedEof => {
                    return Ok(None);
                }
                Err(error) => return Err(format!("unreadable {name} stream ({error})")),
            };
            if packet.track_id != self.track_id {
                continue;
            }
            let buffer = match self.decoder.decode(&packet) {
                Ok(buffer) => buffer,
                Err(Error::DecodeError(_)) => continue,
                Err(error) => return Err(format!("undecodable {name} stream ({error})")),
            };
            let (rate, channels) = (buffer.spec().rate(), buffer.spec().channels().count());
            match self.spec {
                None => {
                    if !matches!(channels, 1 | 2) || rate == 0 {
                        return Err(format!(
                            "unsupported {name} layout: {channels} channels at {rate} Hz"
                        ));
                    }
                    self.spec = Some((rate, channels));
                }
                Some(known) if known != (rate, channels) => {
                    return Err(format!("{name} stream changes its rate or channels"));
                }
                Some(_) => {}
            }
            let mut samples = vec![0.0; buffer.samples_interleaved()];
            buffer.copy_to_slice_interleaved(&mut samples);
            return Ok(Some(Packet {
                rate,
                channels: channels as u16,
                samples,
            }));
        }
    }
}

fn guarded<T>(
    container: Container,
    read: impl FnOnce() -> Result<T, String> + std::panic::UnwindSafe,
) -> Result<T, String> {
    std::panic::catch_unwind(read)
        .unwrap_or_else(|_| Err(format!("the {} decoder failed on it", container.name())))
}

fn read<B: AsRef<[u8]> + Send + Sync + std::panic::UnwindSafe + 'static>(bytes: B, container: Container, first_only: bool) -> Result<Pcm, String> {
    let mut stream = guarded(container, || Stream::open(bytes, container))?;
    let mut samples = Vec::new();
    let mut spec = None;
    while let Some(packet) = stream.decode_next()? {
        spec = Some((packet.rate, packet.channels));
        samples.extend_from_slice(&packet.samples);
        if first_only {
            break;
        }
    }
    let (rate, channels) =
        spec.ok_or_else(|| format!("{} stream without audio", container.name()))?;
    let format = match container {
        Container::Wave => crate::wav::Format::S16,
        Container::Ogg => crate::wav::Format::F32,
    };
    Ok(Pcm {
        rate,
        channels,
        format,
        samples,
    })
}

#[cfg(test)]
mod tests {
    #[test]
    fn non_ogg_bytes_are_refused_by_name() {
        let error = super::decode(b"RIFF....WAVE".to_vec()).unwrap_err();
        assert!(error.starts_with("not an Ogg Vorbis stream"), "{error}");
        assert!(super::decode(Vec::new()).is_err());
    }

    #[test]
    fn garbage_ogg_streams_are_refused_without_panicking() {
        let mut bytes = b"OggS".to_vec();
        bytes.extend((0..4096u32).map(|index| (index.wrapping_mul(2_654_435_761) >> 24) as u8));
        for end in [4, 5, 27, 28, 64, 300, bytes.len()] {
            assert!(super::decode(bytes[..end].to_vec()).is_err());
            assert!(super::probe(bytes[..end].to_vec()).is_err());
        }
    }
}
