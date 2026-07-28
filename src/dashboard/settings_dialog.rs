use gpui::{
    AnyElement, Context, CursorStyle, FontWeight, KeyDownEvent, SharedString, deferred, div,
    prelude::*, px, rgb, rgba, svg,
};

use crate::theme::{ThemeMode, set_reduce_motion, set_theme, theme};

use super::{
    Dashboard,
    components::{secondary_button, section_note, segmented, setting_row, toggle_switch},
};

const DIALOG_WIDTH: f32 = 760.0;
const DIALOG_HEIGHT: f32 = 512.0;
const SIDEBAR_WIDTH: f32 = 198.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SettingsSection {
    General,
    Appearance,
    Data,
    About,
}

impl SettingsSection {
    const ALL: [Self; 4] = [Self::General, Self::Appearance, Self::Data, Self::About];

    fn label(self) -> &'static str {
        match self {
            Self::General => "General",
            Self::Appearance => "Appearance",
            Self::Data => "Your data",
            Self::About => "About",
        }
    }

    fn icon(self) -> &'static str {
        match self {
            Self::General => "settings.svg",
            Self::Appearance => "appearance.svg",
            Self::Data => "data.svg",
            Self::About => "info.svg",
        }
    }

    fn element_id(self) -> &'static str {
        match self {
            Self::General => "settings-general",
            Self::Appearance => "settings-appearance",
            Self::Data => "settings-data",
            Self::About => "settings-about",
        }
    }
}

impl Dashboard {
    pub(super) fn open_settings(&mut self, cx: &mut Context<Self>) {
        self.settings_section = Some(SettingsSection::General);
        cx.notify();
    }

    pub(super) fn close_settings(&mut self, cx: &mut Context<Self>) {
        if self.settings_section.take().is_some() {
            cx.notify();
        }
    }

    fn apply_theme(&mut self, mode: ThemeMode, cx: &mut Context<Self>) {
        if self.preferences.theme == mode {
            return;
        }
        self.preferences.theme = mode;
        set_theme(mode);
        self.persist_preferences(cx);
        self.accounts.update(cx, |_, cx| cx.notify());
        cx.notify();
    }

    fn apply_startup_preference(&mut self, cx: &mut Context<Self>) {
        let enabled = self.preferences.start_with_windows;
        let hidden = self.preferences.start_hidden;
        match crate::settings::set_start_with_windows(enabled, hidden) {
            Ok(()) => tracing::info!(enabled, hidden, "startup entry updated"),
            Err(error) => {
                tracing::error!(reason = %error, "startup entry could not be updated");
                self.preferences.start_with_windows = crate::settings::starts_with_windows();
            }
        }
        self.persist_preferences(cx);
        cx.notify();
    }

    fn apply_start_menu_preference(&mut self, cx: &mut Context<Self>) {
        let enabled = self.preferences.show_in_start_menu;
        match crate::settings::set_shows_in_start_menu(enabled) {
            Ok(()) => tracing::info!(enabled, "start menu shortcut updated"),
            Err(error) => {
                tracing::error!(reason = %error, "start menu shortcut could not be updated");
                self.preferences.show_in_start_menu = crate::settings::shows_in_start_menu();
            }
        }
        self.persist_preferences(cx);
        cx.notify();
    }

    pub(super) fn render_settings_dialog(&mut self, cx: &mut Context<Self>) -> AnyElement {
        let Some(section) = self.settings_section else {
            return div().into_any_element();
        };

        deferred(
            div()
                .id("settings-modal")
                .absolute()
                .top_0()
                .left_0()
                .size_full()
                .bg(rgba(theme().scrim))
                .occlude()
                .on_key_down(cx.listener(|this, event: &KeyDownEvent, _, cx| {
                    if event.keystroke.key.as_str() == "escape" {
                        this.close_settings(cx);
                    }
                    cx.stop_propagation();
                }))
                .flex()
                .items_center()
                .justify_center()
                .child(
                    div()
                        .w(px(DIALOG_WIDTH))
                        .h(px(DIALOG_HEIGHT))
                        .rounded(px(16.0))
                        .border_1()
                        .border_color(rgb(theme().strong_border))
                        .bg(rgb(theme().surface))
                        .overflow_hidden()
                        .flex()
                        .child(self.render_sidebar(section, cx))
                        .child(self.render_pane(section, cx)),
                ),
        )
        .with_priority(100)
        .into_any_element()
    }

    fn render_sidebar(&self, section: SettingsSection, cx: &mut Context<Self>) -> AnyElement {
        div()
            .w(px(SIDEBAR_WIDTH))
            .h_full()
            .flex_none()
            .bg(rgb(theme().inset_surface))
            .border_r_1()
            .border_color(rgb(theme().divider))
            .flex()
            .flex_col()
            .child(
                div()
                    .h(px(64.0))
                    .flex_none()
                    .px(px(18.0))
                    .flex()
                    .items_center()
                    .text_size(px(14.5))
                    .font_weight(FontWeight::SEMIBOLD)
                    .text_color(rgb(theme().text_primary))
                    .child("Settings"),
            )
            .child(
                div()
                    .flex_1()
                    .px(px(10.0))
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .children(
                        SettingsSection::ALL
                            .into_iter()
                            .map(|item| self.render_sidebar_item(item, item == section, cx))
                            .collect::<Vec<_>>(),
                    ),
            )
            .into_any_element()
    }

    fn render_sidebar_item(
        &self,
        item: SettingsSection,
        selected: bool,
        cx: &mut Context<Self>,
    ) -> AnyElement {
        div()
            .id(item.element_id())
            .w_full()
            .h(px(34.0))
            .px(px(10.0))
            .rounded(px(8.0))
            .flex()
            .items_center()
            .gap(px(10.0))
            .cursor(CursorStyle::PointingHand)
            .when(selected, |row| row.bg(rgb(theme().sidebar_selected)))
            .when(!selected, |row| {
                row.hover(|style| style.bg(rgb(theme().sidebar_hover)))
            })
            .on_click(cx.listener(move |this, _, _, cx| {
                this.settings_section = Some(item);
                cx.notify();
            }))
            .child(
                svg()
                    .path(item.icon())
                    .size(px(15.0))
                    .flex_none()
                    .text_color(rgb(if selected {
                        theme().text_primary
                    } else {
                        theme().text_tertiary
                    })),
            )
            .child(
                div()
                    .text_size(px(12.5))
                    .font_weight(if selected {
                        FontWeight::MEDIUM
                    } else {
                        FontWeight::NORMAL
                    })
                    .text_color(rgb(if selected {
                        theme().text_primary
                    } else {
                        theme().text_secondary
                    }))
                    .child(item.label()),
            )
            .into_any_element()
    }

    fn render_pane(&self, section: SettingsSection, cx: &mut Context<Self>) -> AnyElement {
        let body = match section {
            SettingsSection::General => self.render_general(cx),
            SettingsSection::Appearance => self.render_appearance(cx),
            SettingsSection::Data => self.render_data(cx),
            SettingsSection::About => self.render_about(cx),
        };

        div()
            .flex_1()
            .h_full()
            .min_w(px(0.0))
            .flex()
            .flex_col()
            .child(
                div()
                    .h(px(64.0))
                    .flex_none()
                    .px(px(28.0))
                    .border_b_1()
                    .border_color(rgb(theme().divider))
                    .flex()
                    .items_center()
                    .justify_between()
                    .child(
                        div()
                            .text_size(px(14.0))
                            .font_weight(FontWeight::SEMIBOLD)
                            .text_color(rgb(theme().text_primary))
                            .child(section.label()),
                    )
                    .child(
                        div()
                            .id("settings-close")
                            .size(px(28.0))
                            .rounded(px(8.0))
                            .flex()
                            .items_center()
                            .justify_center()
                            .cursor(CursorStyle::PointingHand)
                            .hover(|style| style.bg(rgb(theme().sidebar_hover)))
                            .on_click(cx.listener(|this, _, _, cx| this.close_settings(cx)))
                            .child(
                                svg()
                                    .path("close.svg")
                                    .size(px(14.0))
                                    .text_color(rgb(theme().text_secondary)),
                            ),
                    ),
            )
            .child(
                div()
                    .id("settings-pane")
                    .flex_1()
                    .w_full()
                    .overflow_y_scroll()
                    .px(px(28.0))
                    .py(px(6.0))
                    .child(body),
            )
            .into_any_element()
    }

    fn render_general(&self, cx: &mut Context<Self>) -> AnyElement {
        let start_with_windows = self.preferences.start_with_windows;
        let start_hidden = self.preferences.start_hidden;
        let show_in_start_menu = self.preferences.show_in_start_menu;
        let multi = self.preferences.multiple_instances_enabled;

        div()
            .w_full()
            .flex()
            .flex_col()
            .child(divided(setting_row(
                "Start with Windows",
                "Opens Multiple Roblox automatically when you sign in.",
                toggle_switch(
                    "setting-start-with-windows",
                    start_with_windows,
                    cx.listener(|this, _, _, cx| {
                        this.preferences.start_with_windows = !this.preferences.start_with_windows;
                        this.apply_startup_preference(cx);
                    }),
                ),
            )))
            .child(divided(setting_row(
                "Start hidden",
                "Comes up in the notification area instead of opening the window.",
                toggle_switch(
                    "setting-start-hidden",
                    start_hidden,
                    cx.listener(|this, _, _, cx| {
                        this.preferences.start_hidden = !this.preferences.start_hidden;
                        this.apply_startup_preference(cx);
                    }),
                ),
            )))
            .child(divided(setting_row(
                "Show in Start menu",
                "Adds a shortcut so searching the Start menu finds Multiple Roblox.",
                toggle_switch(
                    "setting-start-menu",
                    show_in_start_menu,
                    cx.listener(|this, _, _, cx| {
                        this.preferences.show_in_start_menu = !this.preferences.show_in_start_menu;
                        this.apply_start_menu_preference(cx);
                    }),
                ),
            )))
            .child(setting_row(
                "Enable multiple Roblox",
                "Holds the Roblox single-client lock so several accounts can run at once.",
                toggle_switch(
                    "setting-multi-instance",
                    multi,
                    cx.listener(|this, _, _, cx| this.toggle_multiple_instances(cx)),
                ),
            ))
            .into_any_element()
    }

    fn render_appearance(&self, cx: &mut Context<Self>) -> AnyElement {
        let reduce_motion = self.preferences.reduce_motion;
        let mode = self.preferences.theme;

        div()
            .w_full()
            .flex()
            .flex_col()
            .child(divided(setting_row(
                "Theme",
                "Changes the whole interface straight away.",
                segmented(
                    "setting-theme",
                    &[
                        (ThemeMode::Dark, ThemeMode::Dark.label()),
                        (ThemeMode::Light, ThemeMode::Light.label()),
                    ],
                    mode,
                    cx.listener(|this, mode: &ThemeMode, _, cx| this.apply_theme(*mode, cx)),
                ),
            )))
            .child(setting_row(
                "Reduce motion",
                "Turns off the highlight that plays when an account is added.",
                toggle_switch(
                    "setting-reduce-motion",
                    reduce_motion,
                    cx.listener(|this, _, _, cx| {
                        this.preferences.reduce_motion = !this.preferences.reduce_motion;
                        set_reduce_motion(this.preferences.reduce_motion);
                        this.persist_preferences(cx);
                        cx.notify();
                    }),
                ),
            ))
            .into_any_element()
    }
}

impl Dashboard {
    fn render_data(&self, cx: &mut Context<Self>) -> AnyElement {
        let clearable = crate::settings::clearable_bytes();
        let accounts = crate::settings::stored_account_count(self.services.vault());
        let confirming = self.confirming_reset;

        div()
            .w_full()
            .flex()
            .flex_col()
            .child(setting_row(
                "Clear cached files",
                SharedString::from(format!(
                    "Leftover sign-in browser data and old logs. Currently {}.",
                    crate::settings::format_bytes(clearable)
                )),
                secondary_button(
                    "clear-cache",
                    "Clear",
                    88.0,
                    cx.listener(|this, _, _, cx| this.clear_cached_files(cx)),
                ),
            ))
            .child(
                div()
                    .mt(px(26.0))
                    .pt(px(18.0))
                    .border_t_1()
                    .border_color(rgb(theme().divider))
                    .flex()
                    .flex_col()
                    .child(
                        div()
                            .text_size(px(12.5))
                            .font_weight(FontWeight::MEDIUM)
                            .text_color(rgb(theme().warning_text))
                            .child("Delete everything"),
                    )
                    .child(section_note(SharedString::from(format!(
                        "Removes {} saved sign-in{}, your favourites and settings, and every \
                         cached file, then closes the app. This cannot be undone.",
                        accounts,
                        if accounts == 1 { "" } else { "s" }
                    ))))
                    .child(
                        div()
                            .mt(px(14.0))
                            .flex()
                            .items_center()
                            .gap(px(10.0))
                            .when(!confirming, |row| {
                                row.child(danger_button(
                                    "reset-begin",
                                    "Delete all my data",
                                    156.0,
                                    cx.listener(|this, _, _, cx| {
                                        this.confirming_reset = true;
                                        cx.notify();
                                    }),
                                ))
                            })
                            .when(confirming, |row| {
                                row.child(
                                    div()
                                        .flex_1()
                                        .text_size(px(11.5))
                                        .text_color(rgb(theme().warning_text))
                                        .child("This is permanent. Continue?"),
                                )
                                .child(secondary_button(
                                    "reset-cancel",
                                    "Cancel",
                                    88.0,
                                    cx.listener(|this, _, _, cx| {
                                        this.confirming_reset = false;
                                        cx.notify();
                                    }),
                                ))
                                .child(danger_button(
                                    "reset-confirm",
                                    "Delete and quit",
                                    132.0,
                                    cx.listener(|this, _, _, cx| this.delete_everything(cx)),
                                ))
                            }),
                    ),
            )
            .into_any_element()
    }

    fn clear_cached_files(&mut self, cx: &mut Context<Self>) {
        match crate::settings::clear_cached_files() {
            Ok(freed) => tracing::info!(freed, "cached files cleared at the user's request"),
            Err(error) => tracing::error!(reason = %error, "cached files could not be cleared"),
        }
        cx.notify();
    }

    fn delete_everything(&mut self, cx: &mut Context<Self>) {
        if let Err(error) = crate::settings::delete_everything(self.services.vault()) {
            tracing::error!(reason = %error, "data could not be fully deleted");
        }
        self.multi_instance = None;
        tracing::info!("local data deleted; exiting");
        cx.quit();
    }
}

pub(super) enum UpdateState {
    Idle,
    Checking,
    Current,
    Available(crate::update::Release),
    Installing,
    Failed(SharedString),
}

impl Dashboard {
    fn check_for_update(&mut self, cx: &mut Context<Self>) {
        if matches!(
            self.update_state,
            UpdateState::Checking | UpdateState::Installing
        ) {
            return;
        }

        self.update_state = UpdateState::Checking;
        cx.notify();

        let task =
            cx.background_spawn(async move { crate::update::check(env!("CARGO_PKG_VERSION")) });

        cx.spawn(async move |this, cx| {
            let result = task.await;
            let _ = this.update(cx, |this, cx| {
                this.update_state = match result {
                    Ok(Some(release)) => UpdateState::Available(release),
                    Ok(None) => UpdateState::Current,
                    Err(error) => {
                        tracing::warn!(reason = %error, "update check failed");
                        UpdateState::Failed(SharedString::from(error.to_string()))
                    }
                };
                cx.notify();
            });
        })
        .detach();
    }

    fn install_update(&mut self, cx: &mut Context<Self>) {
        let UpdateState::Available(release) = &self.update_state else {
            return;
        };
        let url = release.download_url.clone();

        self.update_state = UpdateState::Installing;
        cx.notify();

        let task = cx.background_spawn(async move {
            let installed = crate::update::install(&url)?;
            crate::update::relaunch(&installed)
        });

        cx.spawn(async move |this, cx| match task.await {
            Ok(()) => {
                let _ = cx.update(|cx| cx.quit());
            }
            Err(error) => {
                let _ = this.update(cx, |this, cx| {
                    tracing::error!(reason = %error, "update could not be installed");
                    this.update_state = UpdateState::Failed(SharedString::from(error.to_string()));
                    cx.notify();
                });
            }
        })
        .detach();
    }

    fn render_update_row(&self, cx: &mut Context<Self>) -> AnyElement {
        let busy = matches!(
            self.update_state,
            UpdateState::Checking | UpdateState::Installing
        );

        let status: Option<SharedString> = match &self.update_state {
            UpdateState::Idle => None,
            UpdateState::Checking => Some("Checking".into()),
            UpdateState::Current => Some("You are on the latest version".into()),
            UpdateState::Available(release) => {
                Some(format!("Update available ({})", release.version).into())
            }
            UpdateState::Installing => Some("Downloading and restarting".into()),
            UpdateState::Failed(reason) => Some(reason.clone()),
        };

        let failed = matches!(self.update_state, UpdateState::Failed(_));
        let updatable = matches!(self.update_state, UpdateState::Available(_));

        div()
            .mt(px(20.0))
            .w_full()
            .flex()
            .items_center()
            .justify_between()
            .gap(px(12.0))
            .child(
                div()
                    .flex_1()
                    .min_w(px(0.0))
                    .flex()
                    .flex_col()
                    .gap(px(2.0))
                    .child(
                        div()
                            .text_size(px(12.5))
                            .text_color(rgb(theme().text_primary))
                            .child("Updates"),
                    )
                    .when_some(status, |block, status| {
                        block.child(
                            div()
                                .text_size(px(11.5))
                                .text_color(rgb(if failed {
                                    theme().danger_text
                                } else {
                                    theme().text_tertiary
                                }))
                                .child(status),
                        )
                    }),
            )
            .child(
                div()
                    .flex_none()
                    .flex()
                    .items_center()
                    .gap(px(8.0))
                    .child(secondary_button(
                        "settings-check-update",
                        if busy { "Working" } else { "Check for updates" },
                        150.0,
                        cx.listener(|this, _, _, cx| this.check_for_update(cx)),
                    ))
                    .when(updatable, |actions| {
                        actions.child(secondary_button(
                            "settings-install-update",
                            "Update",
                            92.0,
                            cx.listener(|this, _, _, cx| this.install_update(cx)),
                        ))
                    }),
            )
            .into_any_element()
    }

    fn render_about(&self, cx: &mut Context<Self>) -> AnyElement {
        div()
            .w_full()
            .flex()
            .flex_col()
            .child(render_about_header())
            .child(self.render_update_row(cx))
            .into_any_element()
    }
}

fn render_about_header() -> AnyElement {
    div()
        .flex()
        .flex_col()
        .child(
            div()
                .mt(px(14.0))
                .flex()
                .items_center()
                .gap(px(12.0))
                .child(
                    svg()
                        .path("multiple.svg")
                        .size(px(30.0))
                        .flex_none()
                        .text_color(rgb(theme().text_primary)),
                )
                .child(
                    div()
                        .flex()
                        .flex_col()
                        .child(
                            div()
                                .text_size(px(14.0))
                                .font_weight(FontWeight::SEMIBOLD)
                                .text_color(rgb(theme().text_primary))
                                .child("Multiple Roblox"),
                        )
                        .child(
                            div()
                                .mt(px(2.0))
                                .text_size(px(11.5))
                                .text_color(rgb(theme().text_tertiary))
                                .child(SharedString::from(format!(
                                    "Version {}",
                                    env!("CARGO_PKG_VERSION")
                                ))),
                        ),
                ),
        )
        .into_any_element()
}

fn danger_button(
    id: &'static str,
    label: &'static str,
    width: f32,
    on_click: impl Fn(&gpui::ClickEvent, &mut gpui::Window, &mut gpui::App) + 'static,
) -> AnyElement {
    div()
        .id(id)
        .h(px(34.0))
        .w(px(width))
        .flex_none()
        .rounded(px(9.0))
        .border_1()
        .border_color(rgb(theme().warning_border))
        .bg(rgb(theme().warning_surface))
        .text_color(rgb(theme().warning_strong_text))
        .text_size(px(11.5))
        .font_weight(FontWeight::MEDIUM)
        .cursor(CursorStyle::PointingHand)
        .focusable()
        .hover(|style| {
            style
                .bg(rgb(theme().warning_hover))
                .border_color(rgb(theme().warning_hover_border))
        })
        .active(|style| style.bg(rgb(theme().warning_pressed)))
        .flex()
        .items_center()
        .justify_center()
        .on_click(on_click)
        .child(label)
        .into_any_element()
}

fn divided(row: AnyElement) -> AnyElement {
    div()
        .w_full()
        .border_b_1()
        .border_color(rgb(theme().divider))
        .child(row)
        .into_any_element()
}
