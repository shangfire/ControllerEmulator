#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
mod driver;
mod gui;
mod runtime;
mod scripts;
fn main() {
    if let Err(error) = gui::run() {
        eprintln!("{error}");
    }
}
