//! Recorded economic rows and their typed table operations.
use engine_allocation as allocation;
use replay_rows::codec::{Decoder, invalid_static as invalid, put_varint};
use replay_rows::rows::*;
use replay_rows::typed_codec;
use std::io;

pub use schema::*;
mod schema;
