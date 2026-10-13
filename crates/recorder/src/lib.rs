//! Passive recording, finalization and offline inspection. The `recorder`
//! binary is the command line; [`controller`] is the small API the tray
//! recorder drives in-process.
pub mod controller;
mod derived;
mod metrics;
mod movement_oracle;
mod oracle;
pub mod recording;
mod replay_io;
mod sizes;
#[cfg(test)]
mod test_support;
mod windows;
