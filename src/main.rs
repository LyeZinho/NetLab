//! NetworksCraft desktop entry point.
//!
//! `--screenshot <file.png>` draws 120 frames, saves the window picture and exits.
//! `--self-test <file.json>` runs the headless player checks and exits.

#![deny(
    clippy::unwrap_used,
    clippy::expect_used,
    clippy::panic,
    clippy::unimplemented,
    clippy::todo,
    clippy::unreachable
)]
#![forbid(unsafe_code)]

use eframe::egui;
use networkscraft::engine::Session;
use networkscraft::theme;
use networkscraft::ui::{App, SCREENSHOT_STATE};
use std::sync::atomic::Ordering;

struct Shell {
    app: App,
}

impl eframe::App for Shell {
    fn ui(&mut self, ui: &mut egui::Ui, _frame: &mut eframe::Frame) {
        self.app.draw(ui);
        self.app.tick_screenshot(ui.ctx());
    }
}

fn print_help() {
    println!(
        r#"NetworksCraft — education-first network lab simulator

Usage:
  networkscraft                      open the lab editor
  networkscraft --screenshot FILE    draw 120 frames, save a PNG, exit
  networkscraft --self-test FILE     run headless checks, save JSON, exit

Terminal commands: help, sample, adddevice host PC1, link 0 1,
setipv4 1 10.0.0.1, select 1, ping 10.0.0.2, openlab FILE, savelab FILE"#
    );
}
fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut screenshot: Option<String> = None;
    let mut self_test: Option<String> = None;
    let mut screenshot_flag = false;
    let mut self_test_flag = false;
    let mut index = 0usize;
    while index < args.len() {
        match args.get(index).map(String::as_str) {
            Some("--screenshot") => {
                screenshot_flag = true;
                screenshot = args.get(index.saturating_add(1)).cloned();
                index = index.saturating_add(2);
            }
            Some("--self-test") => {
                self_test_flag = true;
                self_test = args.get(index.saturating_add(1)).cloned();
                index = index.saturating_add(2);
            }
            Some("--help" | "-h") => {
                print_help();
                return Ok(());
            }
            _ => index = index.saturating_add(1),
        }
    }

    if self_test_flag && self_test.is_none() {
        eprintln!("--self-test needs an output path");
        std::process::exit(2);
    }
    if screenshot_flag && screenshot.is_none() {
        eprintln!("--screenshot needs an output path");
        std::process::exit(2);
    }

    if let Some(path) = self_test {
        return match networkscraft::selftest::run(&path) {
            Ok(()) => {
                println!("self-test passed: {path}");
                Ok(())
            }
            Err(error) => {
                eprintln!("self-test failed: {error}");
                std::process::exit(1);
            }
        };
    }

    if let Some(path) = &screenshot
        && path.trim().is_empty()
    {
        eprintln!("--screenshot needs an output path");
        std::process::exit(2);
    }

    let is_screenshot = screenshot.is_some();
    let shot_for_app = screenshot.clone();
    let options = eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_title("NetworksCraft — Network Lab Simulator")
            .with_inner_size([1440.0, 900.0])
            .with_min_inner_size([1080.0, 680.0]),
        renderer: eframe::Renderer::Glow,
        centered: true,
        ..Default::default()
    };

    let result = eframe::run_native(
        "NetworksCraft",
        options,
        Box::new(move |cc| {
            theme::install(&cc.egui_ctx);
            let session = Session::sample();
            Ok(Box::new(Shell {
                app: App::new(session, shot_for_app),
            }))
        }),
    );

    if is_screenshot {
        let state = SCREENSHOT_STATE.load(Ordering::Relaxed);
        if state != 1 {
            eprintln!("screenshot was not saved");
            std::process::exit(1);
        }
        if let Some(path) = &screenshot {
            println!("screenshot saved: {path}");
        }
    }

    result
}
