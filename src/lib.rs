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

pub mod accounts;
pub mod capture;
pub mod config;
pub mod control;
pub mod debugbus;
pub mod flows;
pub mod game;
pub mod geom;
pub mod gui;
pub mod input;
pub mod keys;
pub mod layout;
pub mod movement;
pub mod multi;
pub mod placement;
pub mod platform;
pub mod rbx;
pub mod robloxlog;
pub mod secrets;
pub mod spotmem;
pub mod update;
pub mod vision;
pub mod webhook;
