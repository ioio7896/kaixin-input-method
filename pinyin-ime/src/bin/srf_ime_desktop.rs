#![cfg_attr(windows, windows_subsystem = "windows")]

#[path = "srf_ime_desktop/clipboard.rs"]
mod clipboard;
#[path = "../fonts.rs"]
mod fonts;
#[path = "srf_ime_desktop/handwrite.rs"]
mod handwrite;
#[path = "srf_ime_desktop/keyboard.rs"]
mod keyboard;
#[path = "srf_ime_desktop/ocr.rs"]
mod ocr;
#[path = "srf_ime_desktop/settings.rs"]
mod settings;
#[path = "srf_ime_desktop/symbols.rs"]
mod symbols;
#[path = "srf_ime_desktop/tray.rs"]
mod tray;

fn main() -> eframe::Result<()> {
    let first = std::env::args().nth(1).unwrap_or_default();
    let role = first
        .strip_prefix("--tool=")
        .unwrap_or(match first.as_str() {
            "--ocr-worker" => "ocr",
            "--translation-preview"
            | "--screenshot-capture"
            | "--uia-snap-helper"
            | "--capture-frame-helper"
            | "--capture-recovery-check"
            | "--menu-preview" => "tray",
            _ => "settings",
        });
    match role {
        "settings" => settings::run(),
        "tray" => {
            tray::run();
            Ok(())
        }
        "clipboard" => clipboard::run(),
        "handwrite" => handwrite::run(),
        "symbols" => symbols::run(),
        "keyboard" => keyboard::run(),
        "ocr" => ocr::run(),
        _ => {
            eprintln!("unrecognized desktop tool: {role}");
            std::process::exit(2);
        }
    }
}
