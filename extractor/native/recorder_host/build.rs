//! The recorder compiles `replay_decoder` by source path, so it carries the
//! same fold source fingerprint that crate computes; see that build script.
include!("../replay_decoder/build.rs");
