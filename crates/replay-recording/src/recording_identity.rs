use replay_data::{codec::Decoder, identity::{Identity, IdentityCodec}};
use std::io;

pub fn decode_identity(content: &[u8]) -> io::Result<Identity> {
    let mut decoder = Decoder::new(content);
    let identity = Identity::decode(&mut decoder)?;
    decoder.finish()?;
    Ok(identity)
}
