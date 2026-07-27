use std::sync::atomic::{AtomicBool, AtomicU8, Ordering};

use serde::{Deserialize, Serialize};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub(crate) enum ThemeMode {
    #[default]
    Dark,
    Light,
}

impl ThemeMode {
    pub(crate) fn label(self) -> &'static str {
        match self {
            Self::Dark => "Dark",
            Self::Light => "Light",
        }
    }
}

static ACTIVE_MODE: AtomicU8 = AtomicU8::new(0);

pub(crate) fn set_theme(mode: ThemeMode) {
    ACTIVE_MODE.store(
        match mode {
            ThemeMode::Dark => 0,
            ThemeMode::Light => 1,
        },
        Ordering::Relaxed,
    );
}

pub(crate) fn active_mode() -> ThemeMode {
    match ACTIVE_MODE.load(Ordering::Relaxed) {
        1 => ThemeMode::Light,
        _ => ThemeMode::Dark,
    }
}

static REDUCE_MOTION: AtomicBool = AtomicBool::new(false);

pub(crate) fn set_reduce_motion(reduce: bool) {
    REDUCE_MOTION.store(reduce, Ordering::Relaxed);
}

pub(crate) fn reduce_motion() -> bool {
    REDUCE_MOTION.load(Ordering::Relaxed)
}

pub(crate) fn theme() -> &'static Theme {
    match active_mode() {
        ThemeMode::Dark => &DARK,
        ThemeMode::Light => &LIGHT,
    }
}

pub(crate) struct Theme {
    pub(crate) canvas: u32,
    pub(crate) surface: u32,
    pub(crate) inset_surface: u32,
    pub(crate) control_surface: u32,
    pub(crate) dialog_surface: u32,
    pub(crate) row_hover: u32,
    pub(crate) scrim: u32,

    pub(crate) border: u32,
    pub(crate) divider: u32,
    pub(crate) strong_border: u32,
    pub(crate) focus_border: u32,

    pub(crate) text_primary: u32,
    pub(crate) text_secondary: u32,
    pub(crate) text_tertiary: u32,
    pub(crate) text_metadata: u32,

    pub(crate) control_border: u32,
    pub(crate) control_text: u32,
    pub(crate) control_hover: u32,
    pub(crate) control_hover_border: u32,
    pub(crate) control_pressed: u32,

    pub(crate) accent_surface: u32,
    pub(crate) accent_text: u32,
    pub(crate) accent_hover: u32,
    pub(crate) accent_pressed: u32,

    pub(crate) action_idle: u32,
    pub(crate) action_icon: u32,
    pub(crate) launch_hover: u32,
    pub(crate) launch_focus: u32,
    pub(crate) launch_pressed: u32,
    pub(crate) launch_border: u32,
    pub(crate) launch_icon_hover: u32,
    pub(crate) remove_hover: u32,
    pub(crate) remove_focus: u32,
    pub(crate) remove_pressed: u32,
    pub(crate) remove_border: u32,
    pub(crate) remove_icon_hover: u32,

    pub(crate) transition_surface: u32,
    pub(crate) transition_border: u32,
    pub(crate) transition_text: u32,
    pub(crate) transition_icon: u32,
    pub(crate) running_surface: u32,
    pub(crate) running_border: u32,
    pub(crate) running_text: u32,
    pub(crate) running_hover: u32,
    pub(crate) running_hover_border: u32,
    pub(crate) running_pressed: u32,

    pub(crate) warning_text: u32,
    pub(crate) warning_surface: u32,
    pub(crate) warning_border: u32,
    pub(crate) warning_strong_text: u32,
    pub(crate) warning_hover: u32,
    pub(crate) warning_hover_border: u32,
    pub(crate) warning_pressed: u32,
    pub(crate) warning_icon: u32,
    pub(crate) explainer_surface: u32,
    pub(crate) explainer_border: u32,
    pub(crate) explainer_title: u32,
    pub(crate) explainer_body: u32,

    pub(crate) toggle_off: u32,
    pub(crate) toggle_on: u32,
    pub(crate) toggle_knob: u32,

    pub(crate) titlebar_icon: u32,
    pub(crate) titlebar_text: u32,
    pub(crate) caption_glyph: u32,
    pub(crate) caption_hover: u32,
    pub(crate) caption_hover_text: u32,
    pub(crate) caption_pressed: u32,
    pub(crate) caption_close: u32,
    pub(crate) caption_close_pressed: u32,
    pub(crate) caption_close_text: u32,

    pub(crate) card_backdrop: u32,
    pub(crate) card_selected_border: u32,
    pub(crate) star_idle: u32,
    pub(crate) star_active: u32,
    pub(crate) star_backdrop: u32,
    pub(crate) star_backdrop_hover: u32,

    pub(crate) sidebar_selected: u32,
    pub(crate) sidebar_hover: u32,

    pub(crate) dialog_button_hover: u32,
    pub(crate) dialog_button_hover_border: u32,
    pub(crate) dialog_button_pressed: u32,
    pub(crate) emphasis_surface: u32,
    pub(crate) emphasis_border: u32,
    pub(crate) emphasis_text: u32,
    pub(crate) emphasis_focus_border: u32,
    pub(crate) emphasis_hover: u32,
    pub(crate) emphasis_hover_border: u32,
    pub(crate) emphasis_pressed: u32,

    pub(crate) status_surface: u32,
    pub(crate) status_border: u32,
    pub(crate) status_text: u32,
    pub(crate) status_error_surface: u32,
    pub(crate) status_error_border: u32,

    pub(crate) danger_surface: u32,
    pub(crate) danger_border: u32,
    pub(crate) danger_text: u32,
    pub(crate) danger_focus_border: u32,
    pub(crate) danger_hover: u32,
    pub(crate) danger_pressed: u32,

    pub(crate) warning_remove_hover: u32,
    pub(crate) warning_remove_border: u32,
    pub(crate) warning_remove_pressed: u32,
    pub(crate) info_icon: u32,
    pub(crate) info_icon_active: u32,

    pub(crate) tooltip_surface: u32,
    pub(crate) tooltip_border: u32,
    pub(crate) tooltip_text: u32,
    pub(crate) tooltip_warm_text: u32,

    pub(crate) disabled_icon: u32,
    pub(crate) search_focus_border: u32,
    pub(crate) link_highlight: u32,
    pub(crate) spinner_muted: u32,
}

pub(crate) static DARK: Theme = Theme {
    canvas: 0x101214,
    surface: 0x17191c,
    inset_surface: 0x141619,
    control_surface: 0x202327,
    dialog_surface: 0x191b1f,
    row_hover: 0x1a1d20,
    scrim: 0x050607b8,

    border: 0x2b2e33,
    divider: 0x25282d,
    strong_border: 0x3a3e45,
    focus_border: 0x737981,

    text_primary: 0xf3f3f1,
    text_secondary: 0xa1a5ac,
    text_tertiary: 0x72777f,
    text_metadata: 0x898e96,

    control_border: 0x34383f,
    control_text: 0xd5d7da,
    control_hover: 0x272b30,
    control_hover_border: 0x454a52,
    control_pressed: 0x2f343a,

    accent_surface: 0xe6e7e3,
    accent_text: 0x121315,
    accent_hover: 0xf0f1ed,
    accent_pressed: 0xd2d4cf,

    action_idle: 0x181a1d,
    action_icon: 0x9ca2aa,
    launch_hover: 0x242a33,
    launch_focus: 0x1d2228,
    launch_pressed: 0x2d3540,
    launch_border: 0x3b4658,
    launch_icon_hover: 0xc9d6e8,
    remove_hover: 0x2a2224,
    remove_focus: 0x241e20,
    remove_pressed: 0x36272a,
    remove_border: 0x4b363a,
    remove_icon_hover: 0xe0b0b5,

    transition_surface: 0x1c2026,
    transition_border: 0x34404f,
    transition_text: 0xbfc8d5,
    transition_icon: 0xb9c7d9,
    running_surface: 0x202327,
    running_border: 0x3a3e45,
    running_text: 0xd8dadd,
    running_hover: 0x292d33,
    running_hover_border: 0x525861,
    running_pressed: 0x31363d,

    warning_text: 0xd9a1a7,
    warning_surface: 0x231c1e,
    warning_border: 0x4b363a,
    warning_strong_text: 0xe7c9ce,
    warning_hover: 0x2d2427,
    warning_hover_border: 0x5f434a,
    warning_pressed: 0x362b2f,
    warning_icon: 0xa8969b,
    explainer_surface: 0x2b2426,
    explainer_border: 0x49363a,
    explainer_title: 0xf1e3e5,
    explainer_body: 0xc0aeb1,

    toggle_off: 0x2b2f35,
    toggle_on: 0x5c8f6a,
    toggle_knob: 0xf3f3f1,

    titlebar_icon: 0xb9bdc2,
    titlebar_text: 0x92979f,
    caption_glyph: 0xb7bbc0,
    caption_hover: 0x1c1f22,
    caption_hover_text: 0xe1e3e5,
    caption_pressed: 0x25292d,
    caption_close: 0xc42b3a,
    caption_close_pressed: 0xa92330,
    caption_close_text: 0xffffff,

    card_backdrop: 0x22262b,
    card_selected_border: 0x7f8894,
    star_idle: 0xe8e9e7,
    star_active: 0xf0c96a,
    star_backdrop: 0x11141899,
    star_backdrop_hover: 0x111418dd,

    sidebar_selected: 0x24282e,
    sidebar_hover: 0x1d2024,

    dialog_button_hover: 0x282c31,
    dialog_button_hover_border: 0x464b53,
    dialog_button_pressed: 0x30343a,
    emphasis_surface: 0x24272b,
    emphasis_border: 0x50555d,
    emphasis_text: 0xe7e8e6,
    emphasis_focus_border: 0x8c929a,
    emphasis_hover: 0x2d3136,
    emphasis_hover_border: 0x686e77,
    emphasis_pressed: 0x353a40,

    status_surface: 0x1d2126,
    status_border: 0x343943,
    status_text: 0xbcc6d2,
    status_error_surface: 0x2a2023,
    status_error_border: 0x554044,

    danger_surface: 0x67343a,
    danger_border: 0x784148,
    danger_text: 0xf4e7e9,
    danger_focus_border: 0xd5a2a8,
    danger_hover: 0x784047,
    danger_pressed: 0x592d32,

    warning_remove_hover: 0x38272c,
    warning_remove_border: 0x5c4046,
    warning_remove_pressed: 0x422e34,
    info_icon: 0x767b83,
    info_icon_active: 0xdfb0b5,

    tooltip_surface: 0x292c31,
    tooltip_border: 0x3b3f45,
    tooltip_text: 0xe8e9e7,
    tooltip_warm_text: 0xe8dadd,

    disabled_icon: 0x5f646b,
    search_focus_border: 0x6d747d,
    link_highlight: 0x20242a,
    spinner_muted: 0x9aa6b5,
};

pub(crate) static LIGHT: Theme = Theme {
    canvas: 0xf4f4f2,
    surface: 0xffffff,
    inset_surface: 0xfaf9f7,
    control_surface: 0xf1f0ed,
    dialog_surface: 0xffffff,
    row_hover: 0xf6f5f3,
    scrim: 0x2b2c2e66,

    border: 0xe2e0dc,
    divider: 0xebe9e5,
    strong_border: 0xd2cfc9,
    focus_border: 0x8a8f97,

    text_primary: 0x1b1c1e,
    text_secondary: 0x5c6068,
    text_tertiary: 0x83888f,
    text_metadata: 0x6f747b,

    control_border: 0xdcd9d4,
    control_text: 0x33363b,
    control_hover: 0xe9e7e3,
    control_hover_border: 0xc9c6c0,
    control_pressed: 0xdedbd6,

    accent_surface: 0x1f2124,
    accent_text: 0xf7f7f5,
    accent_hover: 0x303337,
    accent_pressed: 0x141517,

    action_idle: 0xfbfaf9,
    action_icon: 0x6b7079,
    launch_hover: 0xe7edf6,
    launch_focus: 0xeef2f8,
    launch_pressed: 0xd8e2f0,
    launch_border: 0xa9bcd6,
    launch_icon_hover: 0x2f5c94,
    remove_hover: 0xfaeaec,
    remove_focus: 0xfdf2f3,
    remove_pressed: 0xf3dade,
    remove_border: 0xdfb2b8,
    remove_icon_hover: 0xa8323f,

    transition_surface: 0xeef2f8,
    transition_border: 0xb2c3da,
    transition_text: 0x33455e,
    transition_icon: 0x4a6489,
    running_surface: 0xf1f0ed,
    running_border: 0xd2cfc9,
    running_text: 0x2c2f33,
    running_hover: 0xe6e4e0,
    running_hover_border: 0xbdb9b2,
    running_pressed: 0xdad7d1,

    warning_text: 0xa1414d,
    warning_surface: 0xfdf3f4,
    warning_border: 0xe3bcc1,
    warning_strong_text: 0x8f3742,
    warning_hover: 0xfae9eb,
    warning_hover_border: 0xd4a3aa,
    warning_pressed: 0xf2dade,
    warning_icon: 0x9a6a71,
    explainer_surface: 0xfdf3f4,
    explainer_border: 0xe3bcc1,
    explainer_title: 0x7d2f39,
    explainer_body: 0x6d4a4f,

    toggle_off: 0xcfcdc8,
    toggle_on: 0x4f8760,
    toggle_knob: 0xffffff,

    titlebar_icon: 0x4a4e55,
    titlebar_text: 0x5c6068,
    caption_glyph: 0x4a4e55,
    caption_hover: 0xe6e4e0,
    caption_hover_text: 0x1b1c1e,
    caption_pressed: 0xd8d5d0,
    caption_close: 0xc42b3a,
    caption_close_pressed: 0xa92330,
    caption_close_text: 0xffffff,

    card_backdrop: 0xeceae6,
    card_selected_border: 0x6f757e,
    star_idle: 0xffffff,
    star_active: 0xd9a417,
    star_backdrop: 0x2b2c2e88,
    star_backdrop_hover: 0x2b2c2ecc,

    sidebar_selected: 0xe9e7e3,
    sidebar_hover: 0xf1efec,

    dialog_button_hover: 0xe9e7e3,
    dialog_button_hover_border: 0xc9c6c0,
    dialog_button_pressed: 0xdedbd6,
    emphasis_surface: 0xf1f0ed,
    emphasis_border: 0xc9c6c0,
    emphasis_text: 0x27292d,
    emphasis_focus_border: 0x8a8f97,
    emphasis_hover: 0xe4e2dd,
    emphasis_hover_border: 0xaeaaa3,
    emphasis_pressed: 0xd6d3cd,

    status_surface: 0xeef2f8,
    status_border: 0xc2cfe2,
    status_text: 0x33455e,
    status_error_surface: 0xfdf3f4,
    status_error_border: 0xe3bcc1,

    danger_surface: 0xb43a49,
    danger_border: 0x9c2f3d,
    danger_text: 0xfff5f6,
    danger_focus_border: 0x8f2a37,
    danger_hover: 0xc2404f,
    danger_pressed: 0x9c2f3d,

    warning_remove_hover: 0xf5e2e5,
    warning_remove_border: 0xd9aeb4,
    warning_remove_pressed: 0xecd0d5,
    info_icon: 0x8a8f97,
    info_icon_active: 0xa1414d,

    tooltip_surface: 0x2f3136,
    tooltip_border: 0x44474d,
    tooltip_text: 0xf7f7f5,
    tooltip_warm_text: 0xf6e4e6,

    disabled_icon: 0xb0b4bb,
    search_focus_border: 0x9aa0a8,
    link_highlight: 0xe6ecf5,
    spinner_muted: 0x7a828d,
};

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn switching_mode_changes_which_palette_resolves() {
        set_theme(ThemeMode::Light);
        assert_eq!(active_mode(), ThemeMode::Light);
        assert_eq!(theme().canvas, LIGHT.canvas);

        set_theme(ThemeMode::Dark);
        assert_eq!(active_mode(), ThemeMode::Dark);
        assert_eq!(theme().canvas, DARK.canvas);
    }

    #[test]
    fn the_two_palettes_are_genuinely_different() {
        assert_ne!(DARK.canvas, LIGHT.canvas);
        assert_ne!(DARK.surface, LIGHT.surface);
        assert_ne!(DARK.text_primary, LIGHT.text_primary);
        assert_ne!(DARK.accent_surface, LIGHT.accent_surface);
    }

    #[test]
    fn text_contrasts_with_the_surface_it_sits_on() {
        fn luminance(colour: u32) -> f32 {
            let red = ((colour >> 16) & 0xff) as f32 / 255.0;
            let green = ((colour >> 8) & 0xff) as f32 / 255.0;
            let blue = (colour & 0xff) as f32 / 255.0;
            0.2126 * red + 0.7152 * green + 0.0722 * blue
        }

        for palette in [&DARK, &LIGHT] {
            let surface = luminance(palette.surface);
            for (name, text) in [
                ("primary", palette.text_primary),
                ("secondary", palette.text_secondary),
                ("tertiary", palette.text_tertiary),
            ] {
                let difference = (luminance(text) - surface).abs();
                assert!(
                    difference > 0.25,
                    "{name} text is too close to the surface it sits on"
                );
            }
        }
    }
}
