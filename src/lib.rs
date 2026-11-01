//! AA macro core library.
//!
//! Layering (top calls down, nothing below `platform` knows about the game):
//!
//! ```text
//! cli / gui
//!   -> instances (one worker + state machine per Roblox client)   [phase 3+]
//!     -> input::Arbiter (the ONLY owner of mouse + keyboard)
//!     -> capture (per-window frames, works when covered)
//!       -> platform::Platform (Win32 on Windows, Null elsewhere, Mock in tests)
//! ```

pub mod capture;
pub mod config;
pub mod control;
pub mod geom;
pub mod input;
pub mod keys;
pub mod layout;
pub mod platform;
pub mod vision;
