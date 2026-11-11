#![cfg_attr(all(windows, not(debug_assertions)), windows_subsystem = "windows")]

// Keep these includes at crate scope so the existing private items retain their visibility.
include!("core.rs");
include!("player_app.rs");
include!("eframe_app.rs");
include!("helpers.rs");
include!("tests.rs");
