use gpui::{
    App, AppContext, Application, Bounds, WindowBackgroundAppearance, WindowBounds, WindowOptions,
    px, size,
};
use raw_window_handle::{HasWindowHandle as _, RawWindowHandle};

use crate::{assets::Assets, dashboard::Dashboard, tray};

pub(crate) fn run() {
    tracing::info!("starting application event loop");
    Application::new().with_assets(Assets).run(|cx: &mut App| {
        cx.on_window_closed(|cx| {
            if cx.windows().is_empty() {
                tracing::info!("last application window closed");
                cx.quit();
            }
        })
        .detach();

        let bounds = Bounds::centered(None, size(px(1000.0), px(660.0)), cx);
        let window = cx
            .open_window(
                WindowOptions {
                    window_bounds: Some(WindowBounds::Windowed(bounds)),
                    window_min_size: Some(size(px(860.0), px(540.0))),
                    window_background: WindowBackgroundAppearance::Opaque,
                    app_id: Some("multiple-rblx".into()),
                    titlebar: Some(gpui::TitlebarOptions {
                        title: Some("Multiple Roblox".into()),
                        appears_transparent: true,
                        traffic_light_position: None,
                    }),
                    ..Default::default()
                },
                |_, cx| cx.new(Dashboard::new),
            )
            .expect("opening the application window should succeed");
        tracing::info!("dashboard window opened");

        let _ = window.update(cx, |_, window, cx| {
            if let Ok(handle) = window.window_handle()
                && let RawWindowHandle::Win32(win32) = handle.as_raw()
            {
                tray::remember_main_window(win32.hwnd.get());
            }
            tray::install();

            if crate::settings::launched_hidden() {
                tracing::info!("started hidden; dashboard stays in the notification area");
                tray::hide_main_window();
            }

            window.on_window_should_close(cx, |_, _| {
                if tray::exit_requested() {
                    tracing::info!("quit requested from the tray");
                    return true;
                }
                tracing::debug!("dashboard hidden to the notification area");
                tray::hide_main_window();
                false
            });
        });

        cx.activate(true);
    });
    tracing::info!("application event loop stopped");
}
