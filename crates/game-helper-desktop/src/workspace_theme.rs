//! Shared presentation tokens for the route manager and picker. Probe keeps its own theme.
use gpui_kit::component::{Theme, ThemeMode};
use gpui_kit::{App, Hsla, px, rgb, rgba};

// Surface and content roles for the shared pink workspace.
pub const CANVAS: u32 = 0xFFF7FC;
pub const SURFACE: u32 = 0xFFFFFF;
pub const TEXT: u32 = 0x382D38;
pub const MUTED: u32 = 0x705F70;
pub const BORDER: u32 = 0xE8D5E2;
pub const CONTROL_BORDER: u32 = 0xA17D98;
pub const SELECTED: u32 = 0xFDE5F4;
pub const LIST_SELECTED: u32 = 0xF8C4E4;
pub const LIST_SELECTED_MUTED: u32 = 0x645164;

// The exact primary fill is deliberately separate from readable emphasis/focus.
pub const PRIMARY: u32 = 0xFBA1DB;
pub const PRIMARY_FOREGROUND: u32 = 0x3F2438;
pub const PRIMARY_HOVER: u32 = 0xF78BCF;
pub const PRIMARY_ACTIVE: u32 = 0xEF78C2;
pub const EMPHASIS: u32 = 0x9C3E78;
pub const EMPHASIS_HOVER: u32 = 0x7F3061;
pub const EMPHASIS_ACTIVE: u32 = 0x68254F;
pub const SECONDARY_HOVER: u32 = 0xFBD5ED;
pub const SECONDARY_ACTIVE: u32 = 0xF8C4E4;

// Feedback keeps distinct meaning instead of inheriting the pink primary fill.
pub const SUCCESS: u32 = 0x397052;
pub const SUCCESS_HOVER: u32 = 0x2E6045;
pub const SUCCESS_ACTIVE: u32 = 0x24513A;
pub const WARNING: u32 = 0x8A5A16;
pub const WARNING_HOVER: u32 = 0x754B12;
pub const WARNING_ACTIVE: u32 = 0x623E0D;
pub const DANGER: u32 = 0xA13255;
pub const DANGER_HOVER: u32 = 0x8D2849;
pub const DANGER_ACTIVE: u32 = 0x78203D;
pub const ERROR_BG: u32 = 0xFCEBF1;
pub const SCROLLBAR: u32 = CONTROL_BORDER;
/// RGBA value; use `rgba`, unlike the opaque RGB tokens above.
pub const SCRIM: u32 = 0x382D3866;

pub fn color(value: u32) -> Hsla {
    rgb(value).into()
}

pub fn init(cx: &mut App) {
    Theme::change(ThemeMode::Light, None, cx);
    // Components read resolved background tokens as well as solid text colors.
    // Keep both in sync; mutating global_mut followed by sync_base leaves stale fills.
    Theme::update(cx, |theme| {
        theme.font_family = "Microsoft YaHei UI".into();
        theme.font_size = px(14.);
        theme.mono_font_family = "Cascadia Mono".into();
        theme.mono_font_size = px(13.);
        theme.radius = px(4.);
        theme.radius_lg = px(8.);
        theme.shadow = false;
        let c = &mut theme.colors;
        c.background = color(SURFACE);
        c.foreground = color(TEXT);
        // Component controls use a visible boundary; view separators use BORDER directly.
        c.border = color(CONTROL_BORDER);
        c.input = color(CONTROL_BORDER);
        c.muted = color(CANVAS);
        c.muted_foreground = color(MUTED);
        c.accent = color(SELECTED);
        c.accent_foreground = color(TEXT);

        c.primary = color(PRIMARY);
        c.primary_hover = color(PRIMARY_HOVER);
        c.primary_active = color(PRIMARY_ACTIVE);
        c.primary_foreground = color(PRIMARY_FOREGROUND);
        c.button_primary = c.primary;
        c.button_primary_hover = c.primary_hover;
        c.button_primary_active = c.primary_active;
        c.button_primary_foreground = c.primary_foreground;
        // GPUI Kit derives primary-button borders from `primary`; callers add an
        // EMPHASIS border so this light fill retains a clear control boundary.
        c.button = color(SURFACE);
        c.button_foreground = color(TEXT);
        c.button_hover = color(CANVAS);
        c.button_active = color(SELECTED);
        c.secondary = color(SELECTED);
        c.secondary_foreground = color(TEXT);
        c.secondary_hover = color(SECONDARY_HOVER);
        c.secondary_active = color(SECONDARY_ACTIVE);
        c.button_secondary = c.secondary;
        c.button_secondary_foreground = c.secondary_foreground;
        c.button_secondary_hover = c.secondary_hover;
        c.button_secondary_active = c.secondary_active;

        c.danger = color(DANGER);
        c.danger_foreground = color(SURFACE);
        c.danger_hover = color(DANGER_HOVER);
        c.danger_active = color(DANGER_ACTIVE);
        c.button_danger = c.danger;
        c.button_danger_foreground = c.danger_foreground;
        c.button_danger_hover = c.danger_hover;
        c.button_danger_active = c.danger_active;
        c.warning = color(WARNING);
        c.warning_foreground = color(SURFACE);
        c.warning_hover = color(WARNING_HOVER);
        c.warning_active = color(WARNING_ACTIVE);
        c.button_warning = c.warning;
        c.button_warning_foreground = c.warning_foreground;
        c.button_warning_hover = c.warning_hover;
        c.button_warning_active = c.warning_active;
        c.success = color(SUCCESS);
        c.success_foreground = color(SURFACE);
        c.success_hover = color(SUCCESS_HOVER);
        c.success_active = color(SUCCESS_ACTIVE);
        c.button_success = c.success;
        c.button_success_foreground = c.success_foreground;
        c.button_success_hover = c.success_hover;
        c.button_success_active = c.success_active;
        c.info = color(EMPHASIS);
        c.info_foreground = color(SURFACE);
        c.info_hover = color(EMPHASIS_HOVER);
        c.info_active = color(EMPHASIS_ACTIVE);
        c.button_info = c.info;
        c.button_info_foreground = c.info_foreground;
        c.button_info_hover = c.info_hover;
        c.button_info_active = c.info_active;

        c.ring = color(EMPHASIS);
        c.caret = color(EMPHASIS);
        c.selection = color(SELECTED);
        c.link = color(EMPHASIS);
        c.link_hover = color(EMPHASIS_HOVER);
        c.link_active = color(EMPHASIS_ACTIVE);
        c.popover = color(SURFACE);
        c.popover_foreground = color(TEXT);
        c.overlay = rgba(SCRIM).into();
        c.scrollbar = color(CANVAS);
        c.scrollbar_thumb = color(SCROLLBAR);
        c.scrollbar_thumb_hover = color(EMPHASIS);
        c.list = color(SURFACE);
        c.list_hover = color(CANVAS);
        c.list_active = color(SELECTED);
        c.list_active_border = color(EMPHASIS);
        c.list_even = color(CANVAS);
        c.list_head = color(CANVAS);
        c.sidebar = color(CANVAS);
        c.sidebar_foreground = color(TEXT);
        c.sidebar_border = color(BORDER);
        c.sidebar_accent = color(SELECTED);
        c.sidebar_accent_foreground = color(TEXT);
        c.sidebar_primary = color(PRIMARY);
        c.sidebar_primary_foreground = color(PRIMARY_FOREGROUND);
        c.tab = color(CANVAS);
        c.tab_foreground = color(MUTED);
        c.tab_active = color(SELECTED);
        c.tab_active_foreground = color(TEXT);
        c.tab_bar = color(SURFACE);
        c.tab_bar_segmented = color(CANVAS);
        c.title_bar = color(CANVAS);
        c.title_bar_border = color(BORDER);
        c.status_bar = color(CANVAS);
        c.status_bar_border = color(BORDER);
        c.drag_border = color(EMPHASIS);
        c.drop_target = color(SELECTED);
        c.group_box = color(CANVAS);
        c.group_box_foreground = color(TEXT);
        c.description_list_label = color(CANVAS);
        c.description_list_label_foreground = color(MUTED);
    });
}
