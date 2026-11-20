#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod app;

use sonora_core::{audio, config, lyrics, playlists};

use std::path::PathBuf;
use std::sync::Arc;

fn main() -> eframe::Result {
    env_logger::Builder::from_env(env_logger::Env::default().default_filter_or("info")).init();

    let cli_paths: Vec<PathBuf> = std::env::args_os()
        .skip(1)
        .map(PathBuf::from)
        .filter(|p| p.exists())
        .collect();

    let icon = build_app_icon();

    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_inner_size([1380.0, 860.0])
            .with_min_inner_size([720.0, 560.0])
            .with_decorations(false)
            .with_resizable(true)
            .with_drag_and_drop(true)
            .with_icon(Arc::new(icon))
            .with_title("Sonora Hi-Res Audio Player"),
        ..Default::default()
    };

    eframe::run_native(
        "Sonora Hi-Res Audio Player",
        options,
        Box::new(move |cc| {
            let mut app = app::AudioPlayerApp::new(cc);
            if !cli_paths.is_empty() {
                app.open_initial_paths(cli_paths);
            }
            Ok(Box::new(app))
        }),
    )
}

/// Generate a crisp 64x64 RGBA Swiss modernist high-contrast carbon & Braun orange emblem icon.
fn build_app_icon() -> eframe::egui::IconData {
    let size = 64u32;
    let mut rgba = vec![0u8; (size * size * 4) as usize];
    let c = (size as f32) * 0.5;

    for y in 0..size {
        for x in 0..size {
            let dx = x as f32 + 0.5 - c;
            let dy = y as f32 + 0.5 - c;
            let r = dx.hypot(dy);

            let is_border = x == 1 || x == size - 2 || y == 1 || y == size - 2;
            let (red, grn, blu, alpha) = if x == 0 || x == size - 1 || y == 0 || y == size - 1 {
                (0, 0, 0, 0)
            } else if is_border {
                (58, 58, 68, 255) // 1px hairline outer frame
            } else if (21.5..=23.5).contains(&r) {
                (245, 245, 242, 255) // crisp Swiss white ring
            } else if r < 11.0 {
                (232, 93, 42, 255) // Braun International Orange core
            } else {
                (9, 9, 11, 255) // matte carbon ground
            };

            let idx = ((y * size + x) * 4) as usize;
            rgba[idx] = red;
            rgba[idx + 1] = grn;
            rgba[idx + 2] = blu;
            rgba[idx + 3] = alpha;
        }
    }

    eframe::egui::IconData {
        rgba,
        width: size,
        height: size,
    }
}
