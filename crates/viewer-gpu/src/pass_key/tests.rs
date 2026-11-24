use super::*;
use crate::textures::Content;

fn block(camera: f32, clock: f32) -> Vec<u8> {
    let mut bytes = vec![0u8; 64];
    bytes[..4].copy_from_slice(&camera.to_le_bytes());
    bytes[32..36].copy_from_slice(&clock.to_le_bytes());
    bytes
}

#[expect(clippy::single_range_in_vec_init, reason = "a buffer's reads are a list of byte ranges, here one")]
fn reads() -> shader_pipeline::Buffer {
    shader_pipeline::Buffer {
        binding: 0,
        size: 64,
        reads: vec![0..16],
    }
}

#[expect(clippy::single_range_in_vec_init, reason = "a buffer's reads are a list of byte ranges, here one")]
fn reads_eight() -> shader_pipeline::Buffer {
    shader_pipeline::Buffer {
        binding: 0,
        size: 8,
        reads: vec![0..8],
    }
}

#[test]
fn a_changed_read_byte_redraws_and_a_changed_unread_byte_does_not() {
    let mut keyed = Keyed::default();
    let mut key = keyed.start();
    key.read(&reads(), &block(1.0, 1.0));
    assert!(keyed.draws(key), "nothing holds the target yet");
    keyed.submitted();

    let mut key = keyed.start();
    key.read(&reads(), &block(1.0, 2.0));
    assert!(
        !keyed.draws(key),
        "the clock moved, and the program does not read it"
    );
    keyed.submitted();

    let mut key = keyed.start();
    key.read(&reads(), &block(1.0001, 2.0));
    assert!(keyed.draws(key), "the camera moved, and the program reads it");
}

#[test]
fn reads_past_the_block_are_the_zeros_the_device_reads() {
    let mut keyed = Keyed::default();
    let mut key = keyed.start();
    key.read(&reads_eight(), &[1u8, 2, 3]);
    assert!(keyed.draws(key));
    keyed.submitted();
    let mut key = keyed.start();
    key.read(&reads_eight(), &[1u8, 2, 3, 0, 0, 0, 0, 0]);
    assert!(!keyed.draws(key), "the same bytes, read to the same length");
}

#[test]
fn a_rewritten_texture_redraws_and_a_still_one_does_not() {
    let held = Content::next();
    let mut keyed = Keyed::default();
    let mut key = keyed.start();
    key.content(|| "the sea bed".to_owned(), Some(held));
    assert!(keyed.draws(key));
    keyed.submitted();

    let mut key = keyed.start();
    key.content(|| "the sea bed".to_owned(), Some(held));
    assert!(!keyed.draws(key), "the texture holds what it held");
    keyed.submitted();

    let mut key = keyed.start();
    key.content(|| "the sea bed".to_owned(), Some(Content::next()));
    assert!(keyed.draws(key), "the texture was written");
}

#[test]
fn a_remade_target_redraws() {
    let mut keyed = Keyed::default();
    let mut key = keyed.start();
    key.number(2752).number(1152).content(|| "the sea-bed target".to_owned(), Some(Content::next()));
    assert!(keyed.draws(key));
    keyed.submitted();

    let mut key = keyed.start();
    key.number(2752).number(1152).content(|| "the sea-bed target".to_owned(), Some(Content::next()));
    assert!(keyed.draws(key), "a target of the same size, but another one");
}

#[test]
fn a_draw_list_that_changed_length_redraws() {
    let mut keyed = Keyed::default();
    let mut key = keyed.start();
    key.number(2)
        .read(&reads(), &block(1.0, 0.0))
        .read(&reads(), &block(2.0, 0.0));
    assert!(keyed.draws(key));
    keyed.submitted();

    let mut key = keyed.start();
    key.number(3)
        .read(&reads(), &block(1.0, 0.0))
        .read(&reads(), &block(2.0, 0.0))
        .read(&reads(), &block(3.0, 0.0));
    assert!(keyed.draws(key), "a block more is a different picture");
}

#[test]
fn an_input_no_writer_describes_draws_every_frame() {
    let mut keyed = Keyed::default();
    for _ in 0..3 {
        let mut key = keyed.start();
        key.number(1).content(|| "last frame's shadow buffer".to_owned(), None);
        assert!(keyed.draws(key), "no key can be formed, so the pass draws");
        assert_eq!(
            keyed.unkept(),
            Some("last frame's shadow buffer publishes no content")
        );
        keyed.submitted();
    }
}

#[test]
fn a_frame_that_did_not_reach_the_queue_draws_again() {
    let mut keyed = Keyed::default();
    let mut key = keyed.start();
    key.read(&reads(), &block(1.0, 0.0));
    assert!(keyed.draws(key));
    keyed.submitted();

    let mut key = keyed.start();
    key.read(&reads(), &block(2.0, 0.0));
    assert!(keyed.draws(key), "the camera moved");

    let mut key = keyed.start();
    key.read(&reads(), &block(2.0, 0.0));
    assert!(
        keyed.draws(key),
        "the target still holds the first frame's picture"
    );
    keyed.submitted();

    let mut key = keyed.start();
    key.read(&reads(), &block(2.0, 0.0));
    assert!(!keyed.draws(key));
}

#[test]
fn a_lost_target_draws_again() {
    let mut keyed = Keyed::default();
    let mut key = keyed.start();
    key.read(&reads(), &block(1.0, 0.0));
    assert!(keyed.draws(key));
    keyed.submitted();
    keyed.lost();
    let mut key = keyed.start();
    key.read(&reads(), &block(1.0, 0.0));
    assert!(keyed.draws(key));
}

#[test]
fn the_part_that_differed_is_named() {
    let mut keyed = Keyed::default();
    let mut key = keyed.start();
    key.part("camera").read(&reads(), &block(1.0, 0.0));
    key.part("lights").content(|| "the light tiles".to_owned(), Some(Content::next()));
    assert!(keyed.draws(key));
    assert_eq!(keyed.differed(), None, "the target held nothing to differ from");
    keyed.submitted();

    let light = Content::next();
    let mut key = keyed.start();
    key.part("camera").read(&reads(), &block(1.0, 0.0));
    key.part("lights").content(|| "the light tiles".to_owned(), Some(light));
    assert!(keyed.draws(key));
    assert_eq!(keyed.differed(), Some("lights"));
    keyed.submitted();

    let mut key = keyed.start();
    key.part("camera").read(&reads(), &block(5.0, 0.0));
    key.part("lights").content(|| "the light tiles".to_owned(), Some(light));
    assert!(keyed.draws(key));
    assert_eq!(keyed.differed(), Some("camera"));
}
