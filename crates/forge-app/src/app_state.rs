//! What every window shares: your settings, keyboard shortcuts and saved
//! files in Forge's data folder, and the theme built from them.

use crate::theme::{ForgeTheme, Theme, ThemeName};
use forge_core::commands::Keymap;
use forge_core::settings::Settings;
use forge_sys::user_state::{data_dir, UserState};
use gpui_kit::component::{Theme as KitTheme, ThemeMode};
use gpui_kit::{px, App, Global};
use std::sync::Arc;

pub struct AppState {
    pub updater: Arc<forge_update::Updater>,
    pub restart_after_update: bool,
    pub user: UserState,
    pub settings: Settings,
    /// Why settings.json couldn't be read (the defaults are used).
    pub settings_error: Option<String>,
    pub keymap: Keymap,
    /// Why keybindings.json couldn't be read (the defaults are used).
    pub keys_error: Option<String>,
}

impl Global for AppState {}

pub fn init(cx: &mut App) {
    let user = UserState::new(data_dir());
    let (settings, settings_error) = user.settings();
    let (overrides, keys_error) = user.keybindings();
    let mut keymap = Keymap::default();
    keymap.set_overrides(overrides);
    let updater = Arc::new(forge_update::Updater::new(user.dir().to_path_buf()));
    cx.set_global(AppState {
        user,
        settings,
        settings_error,
        keymap,
        keys_error,
        updater,
        restart_after_update: false,
    });
    apply_theme(cx);
}

pub fn state(cx: &App) -> &AppState {
    cx.global::<AppState>()
}

pub fn settings(cx: &App) -> &Settings {
    &cx.global::<AppState>().settings
}

/// Use the theme the settings ask for, here and in the shared widgets (inputs, menus, dialogs).
pub fn apply_theme(cx: &mut App) {
    let name = match cx.global::<AppState>().settings.theme {
        forge_core::settings::Theme::Molten => ThemeName::Molten,
        forge_core::settings::Theme::Plain => ThemeName::Plain,
    };
    let t = Arc::new(Theme::get(name));
    cx.set_global(ForgeTheme(t.clone()));
    let kit = KitTheme::global_mut(cx);
    kit.mode = ThemeMode::Dark;
    kit.font_family = crate::theme::UI_FONT.into();
    kit.font_size = px(13.0);
    kit.mono_font_family = crate::theme::CODE_FONT.into();
    kit.radius = px(2.0);
    kit.radius_lg = px(3.0);
    let c = &mut kit.colors;
    c.background = t.bg;
    c.foreground = t.fg;
    c.border = t.border;
    c.input = t.line_strong;
    c.ring = t.heat_2;
    c.caret = t.accent;
    c.selection = t.selection;
    c.accent = t.bg_active;
    c.accent_foreground = t.fg;
    c.muted = t.raised;
    c.muted_foreground = t.fg_muted;
    c.popover = t.bg_panel;
    c.popover_foreground = t.fg;
    c.primary = t.heat_3;
    c.primary_hover = t.heat_2;
    c.primary_active = t.heat_2;
    c.primary_foreground = t.on_heat;
    c.secondary = t.raised;
    c.secondary_hover = t.bg_hover;
    c.secondary_active = t.bg_active;
    c.secondary_foreground = t.fg;
    c.list = t.bg_panel;
    c.list_hover = t.bg_hover;
    c.list_active = t.bg_active;
    c.list_active_border = t.heat_2;
    c.list_even = t.bg_panel;
    c.list_head = t.raised;
    c.danger = t.err;
    c.warning = t.warn;
    c.success = t.ok;
    c.info = t.info;
    c.link = t.link;
    c.scrollbar = t.bg_panel;
    c.scrollbar_thumb = t.line_strong;
    c.scrollbar_thumb_hover = t.fg_muted;
    c.overlay = t.overlay;
    c.title_bar = t.bg_panel;
    c.title_bar_border = t.border;
    c.window_border = t.border;
    c.tab = t.bg_panel;
    c.tab_active = t.bg;
    c.tab_bar = t.bg_panel;
    c.tab_foreground = t.fg_muted;
    c.tab_active_foreground = t.fg;
    c.drop_target = t.highlight;
    c.drag_border = t.heat_2;
    cx.refresh_windows();
}

/// Change the settings (and save them): `f` edits a copy.
pub fn update_settings(cx: &mut App, f: impl FnOnce(&mut Settings)) -> std::io::Result<()> {
    let mut next = cx.global::<AppState>().settings.clone();
    f(&mut next);
    let theme_changed = next.theme != cx.global::<AppState>().settings.theme;
    cx.global::<AppState>().user.save_settings(&next)?;
    cx.global_mut::<AppState>().settings = next;
    if theme_changed {
        apply_theme(cx);
    } else {
        cx.refresh_windows();
    }
    Ok(())
}
