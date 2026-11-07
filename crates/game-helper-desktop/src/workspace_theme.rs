//! Shared presentation tokens for the route manager and picker. Probe keeps its own theme.
use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::{App, Hsla, px, rgb};

pub const FOG: u32 = 0xEDF2F4;
pub const PAPER: u32 = 0xFFFFFF;
pub const INK: u32 = 0x173F4A;
pub const TEAL: u32 = 0x236B73;
pub const AMBER: u32 = 0xAA650E;
pub const RED: u32 = 0xB43E4B;
pub const MUTED: u32 = 0x586F78;
pub const LINE: u32 = 0xCFDCDF;
pub const TINT: u32 = 0xDFEEEF;

pub fn color(value: u32) -> Hsla {
    rgb(value).into()
}

pub fn init(cx: &mut App) {
    Theme::change(ThemeMode::Light, None, cx);
    let theme = Theme::global_mut(cx);
    theme.font_family = "Microsoft YaHei UI".into();
    theme.font_size = px(14.);
    theme.mono_font_family = "Cascadia Mono".into();
    theme.mono_font_size = px(13.);
    theme.radius = px(4.);
    theme.radius_lg = px(8.);
    theme.shadow = false;
    let c = &mut theme.colors;
    c.background = color(PAPER);
    c.foreground = color(INK);
    c.border = color(LINE);
    c.input = color(LINE);
    c.muted = color(FOG);
    c.muted_foreground = color(MUTED);
    c.accent = color(TINT);
    c.accent_foreground = color(INK);
    c.primary = color(TEAL);
    c.primary_hover = color(0x1D5A61);
    c.primary_active = color(INK);
    c.primary_foreground = color(PAPER);
    c.button_primary = c.primary;
    c.button_primary_hover = c.primary_hover;
    c.button_primary_active = c.primary_active;
    c.button_primary_foreground = c.primary_foreground;
    c.button = color(PAPER);
    c.button_foreground = color(INK);
    c.button_hover = color(FOG);
    c.button_active = color(TINT);
    c.secondary = color(TINT);
    c.secondary_foreground = color(INK);
    c.secondary_hover = color(0xD1E5E7);
    c.secondary_active = color(0xC0DCDD);
    c.button_secondary = c.secondary;
    c.button_secondary_foreground = c.secondary_foreground;
    c.button_secondary_hover = c.secondary_hover;
    c.button_secondary_active = c.secondary_active;
    c.danger = color(RED);
    c.danger_foreground = color(PAPER);
    c.danger_hover = color(0x963340);
    c.danger_active = color(0x7F2934);
    c.button_danger = c.danger;
    c.button_danger_foreground = c.danger_foreground;
    c.button_danger_hover = c.danger_hover;
    c.button_danger_active = c.danger_active;
    c.warning = color(AMBER);
    c.ring = color(TEAL);
    c.caret = color(TEAL);
    c.selection = color(TINT);
    c.scrollbar_thumb = color(0xACBFC5);
    c.scrollbar_thumb_hover = color(MUTED);
    Theme::sync_base(cx);
}
