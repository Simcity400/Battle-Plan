use crate::{
    Frame,
    engine::{CALLBACK_FRAMES, Engine, Load},
    music,
    system::{Source, decode, guarded, open_track, sample_path},
};
use std::{collections::VecDeque, sync::Arc};

const DEFAULT_SEED: u32 = 0x4155_4449;

pub struct OfflineMixer {
    engine: Engine,
    source: Arc<Source>,
    pending: VecDeque<i16>,
    block: Vec<i16>,
    pub notes: Vec<String>,
}

impl OfflineMixer {
    pub fn new(source: Arc<Source>) -> Self {
        Self::with_seed(source, DEFAULT_SEED)
    }

    pub fn with_seed(source: Arc<Source>, seed: u32) -> Self {
        let mut engine = Engine::new(seed);
        engine.set_content(Some(Arc::clone(&source.content)));
        Self {
            engine,
            source,
            pending: VecDeque::with_capacity(CALLBACK_FRAMES * 4),
            block: vec![0i16; CALLBACK_FRAMES * 2],
            notes: Vec::new(),
        }
    }

    pub fn step_frame(&mut self, frame: &Frame, stereo_frames: usize, out: &mut Vec<i16>) {
        self.engine.frame(frame);
        self.satisfy_loads();
        let needed_samples = stereo_frames * 2;
        while self.pending.len() < needed_samples {
            self.engine.render(&mut self.block);
            self.pending.extend(self.block.iter().copied());
            self.satisfy_loads();
        }
        out.clear();
        out.reserve(needed_samples);
        for _ in 0..needed_samples {
            out.push(self.pending.pop_front().unwrap_or(0));
        }
    }

    pub fn silence_frame(&mut self, stereo_frames: usize, out: &mut Vec<i16>) {
        self.engine.silence();
        self.pending.clear();
        out.clear();
        out.resize(stereo_frames * 2, 0);
    }

    fn satisfy_loads(&mut self) {
        for load in self.engine.take_wanted() {
            match load {
                Load::Sound(sound) => match guarded(&self.source, load, decode) {
                    Ok(samples) => self.engine.provide(sound, Some(samples)),
                    Err(why) => {
                        self.note(why);
                        self.engine.provide(sound, None);
                    }
                },
                Load::Track(track) => match guarded(&self.source, load, open_track) {
                    Ok(mut stream) => {
                        let path = sample_path(&self.source, load)
                            .unwrap_or("an unknown file")
                            .to_owned();
                        let track_stream = music::decode_all(|| {
                            stream
                                .decode_next()
                                .map(|packet| {
                                    packet.map(|p| (p.rate, p.channels, p.samples))
                                })
                                .map_err(|why| format!("{path}: {why}"))
                        });
                        self.engine.provide_track(track, Some(track_stream));
                    }
                    Err(why) => {
                        self.note(why);
                        self.engine.provide_track(track, None);
                    }
                },
            }
        }
    }

    fn note(&mut self, message: String) {
        if !self.notes.contains(&message) {
            self.notes.push(message);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::OfflineMixer;
    use crate::{Frame, engine::Content, system::Source};
    use content_reader_layers::ResolvedLayers;
    use content_reader_sound::sound::Catalog;
    use std::sync::Arc;

    #[test]
    fn offline_mixer_renders_exact_frame_counts_through_whole_callbacks() {
        let source = Arc::new(Source {
            content: Arc::new(Content::new(Catalog::default(), None)),
            layers: ResolvedLayers::default(),
        });
        let mut mixer = OfflineMixer::new(source);
        let frame = Frame::default();
        let mut out = Vec::new();
        let mut total_frames = 0usize;
        for _ in 0..60 {
            mixer.step_frame(&frame, 735, &mut out);
            assert_eq!(out.len(), 735 * 2);
            total_frames += out.len() / 2;
        }
        assert_eq!(total_frames, 44_100);
    }
}
