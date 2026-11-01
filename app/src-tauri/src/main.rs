// Pas de console supplémentaire sous Windows en release.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    norelly_app_lib::run();
}
