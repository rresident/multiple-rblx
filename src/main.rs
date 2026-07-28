#![cfg_attr(
    all(target_os = "windows", not(debug_assertions)),
    windows_subsystem = "windows"
)]

mod accounts;
mod app;
mod app_instance;
mod assets;
mod dashboard;
mod diagnostics;
mod games;
mod launcher;
mod linking;
mod security;
mod settings;
mod theme;
mod tray;
mod update;

fn main() {
    let diagnostics = diagnostics::init();
    if let Some(path) = diagnostics.log_path() {
        tracing::info!(path = %path.display(), "diagnostics are active");
    }
    let _instance = match app_instance::AppInstance::acquire() {
        Ok(Some(instance)) => instance,
        Ok(None) => {
            tracing::info!("another Multiple Roblox process is already running");
            return;
        }
        Err(error) => {
            tracing::error!(reason = %error, "application instance check failed");
            return;
        }
    };
    app::run();
}
