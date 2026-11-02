//! NetworksCraft — an education-first, agent-drivable network lab simulator.
//!
//! The crate is split into layers by module and dependency direction:
//! L0 [`model`] → L2 [`sim`] → L3 [`io`] → L4 [`engine`] → L5 [`ui`] / [`app`].
//! Nothing below the UI layer depends on egui/eframe.

#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]
#![forbid(unsafe_code)]

pub mod engine;
pub mod io;
pub mod model;
pub mod selftest;
pub mod sim;
pub mod theme;
pub mod ui;
