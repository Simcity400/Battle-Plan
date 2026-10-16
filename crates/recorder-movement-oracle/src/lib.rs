//! Read-only native movement census and comparison against the verified content stack's MovementMap. Preserves province/neighbour order and terrain checks; unreadable data and differences fail. Its command opens a game only when explicitly invoked; tests use synthetic maps.
use recorder_platform::{directories, windows};
pub mod movement_oracle;
