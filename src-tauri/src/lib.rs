mod commands;
mod state;
pub(crate) mod tray_icon;

use tauri::{
    tray::{MouseButton, MouseButtonState, TrayIconBuilder, TrayIconEvent},
    Manager, PhysicalPosition, WebviewUrl, WebviewWindowBuilder,
};

#[cfg(target_os = "macos")]
use window_vibrancy::{apply_liquid_glass, NSGlassEffectViewStyle};

#[cfg_attr(mobile, tauri::mobile_entry_point)]
pub fn run() {
    tauri::Builder::default()
        .plugin(tauri_plugin_opener::init())
        .plugin(tauri_plugin_store::Builder::default().build())
        .manage(state::TokenCache::new())
        .manage(state::PayloadCache::new())
        .manage(state::UsagePoller::new())
        .invoke_handler(tauri::generate_handler![
            commands::start_auto_refresh,
            commands::trigger_refresh,
            commands::hide_popup,
            commands::quit_app,
        ])
        .setup(setup_app)
        .on_window_event(handle_window_event)
        .run(tauri::generate_context!())
        .expect("error while running tauri application");
}

fn setup_app(app: &mut tauri::App) -> Result<(), Box<dyn std::error::Error>> {
    #[cfg(target_os = "macos")]
    app.set_activation_policy(tauri::ActivationPolicy::Accessory);

    let handle = app.handle();

    // Generate initial tray icon with empty progress bars
    let icon = tray_icon::generate_icon(0.0, 0.0);

    // Create system tray (no native menu)
    TrayIconBuilder::with_id("main-tray")
        .icon(icon)
        .icon_as_template(false)
        .tooltip("Claude Usage")
        .on_tray_icon_event(|tray, event| {
            if let TrayIconEvent::Click {
                button: MouseButton::Left,
                button_state: MouseButtonState::Up,
                position,
                ..
            } = event
            {
                toggle_popup(tray.app_handle(), position);
            }
        })
        .build(handle)?;

    // Read saved interval from store file, default to 120s
    let interval_secs = read_saved_interval(app).unwrap_or(120);

    // Start native polling timer (immune to WebView throttling)
    app.state::<state::UsagePoller>().restart(handle.clone(), interval_secs);

    Ok(())
}

/// Reads the saved polling interval from the tauri-plugin-store settings file.
fn read_saved_interval(app: &tauri::App) -> Option<u64> {
    let app_data = app.path().app_data_dir().ok()?;
    let settings_path = app_data.join("settings.json");
    let contents = std::fs::read_to_string(settings_path).ok()?;
    let json: serde_json::Value = serde_json::from_str(&contents).ok()?;
    json.get("intervalSeconds")?.as_u64()
}

fn toggle_popup(app: &tauri::AppHandle, click_position: PhysicalPosition<f64>) {
    let showing = if let Some(window) = app.get_webview_window("popup") {
        if window.is_visible().unwrap_or(false) {
            let _ = window.hide();
            false
        } else {
            position_popup(&window, click_position);
            let _ = window.show();
            let _ = window.set_focus();
            true
        }
    } else {
        let window = WebviewWindowBuilder::new(
            app,
            "popup",
            WebviewUrl::App("index.html".into()),
        )
        .title("")
        .inner_size(320.0, 330.0)
        .resizable(false)
        .decorations(false)
        .shadow(false)
        .transparent(true)
        .background_color(tauri::window::Color(0, 0, 0, 0))
        .always_on_top(true)
        .skip_taskbar(true)
        .visible(false)
        .focused(true)
        .build();

        if let Ok(window) = window {
            #[cfg(target_os = "macos")]
            {
                let _ = apply_liquid_glass(
                    &window,
                    NSGlassEffectViewStyle::Clear,
                    Some((20, 20, 25, 180)),
                    Some(12.0),
                );
            }
            position_popup(&window, click_position);
            let _ = window.show();
            let _ = window.set_focus();
            true
        } else {
            false
        }
    };

    // Emit cached payload so the popup gets data immediately
    if showing {
        commands::emit_cached_payload(app);
    }
}

fn position_popup(window: &tauri::WebviewWindow, tray_position: PhysicalPosition<f64>) {
    let popup_width: f64 = 320.0;
    let x = tray_position.x - (popup_width / 2.0);
    let y = tray_position.y + 4.0;

    let _ = window.set_position(tauri::Position::Physical(PhysicalPosition::new(
        x as i32, y as i32,
    )));
}

fn handle_window_event(window: &tauri::Window, event: &tauri::WindowEvent) {
    match event {
        tauri::WindowEvent::CloseRequested { api, .. } => {
            if window.label() == "popup" {
                window.hide().unwrap_or_default();
                api.prevent_close();
            }
        }
        tauri::WindowEvent::Focused(false) => {
            if window.label() == "popup" {
                let _ = window.hide();
            }
        }
        _ => {}
    }
}
