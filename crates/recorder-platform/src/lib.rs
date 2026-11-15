//! Windows process, memory, module and event operations, the choice of the one running game, and installation/user/capture directory defaults. Directory discovery preserves registry and environment search order; capture/finalization policy belongs to recorder. Used by recorder and its comparison tools.
pub mod directories;
pub mod games;
pub mod windows;
