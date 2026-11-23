//! 界面主题：设置中的外观主题（跟随系统 / 浅色 / 深色）解析为当前调色板。
//!
//! 截图工具栏、剪贴板历史、结果窗口、贴图管理等共用 `annotate` 的界面色（经本模块取值），设置窗口另有
//! 页面底色与强调色。解析结果保存在全局原子量中，渲染时直接读取；设置保存或设置窗口重绘时刷新，
//! “跟随系统”按当前系统外观决定。窗口里个别装饰色（标注色板、录屏红框等）不随主题变化。
use gpui_kit::{
    component::{Theme as ComponentTheme, ThemeMode},
    App, WindowAppearance,
};
use snap_core::settings::Theme;
use std::sync::atomic::{AtomicBool, Ordering};

/// 当前是否为深色（旧版本只有深色，默认保持深色直到读取设置）。
static DARK: AtomicBool = AtomicBool::new(true);
/// gpui-component（输入框等）的主题模式是否已同步过一次。
static SYNCED: AtomicBool = AtomicBool::new(false);

pub struct Palette {
    /// 工具栏、卡片等面板底色。
    pub surface: u32,
    pub border: u32,
    pub text: u32,
    pub muted: u32,
    pub hover: u32,
    pub active_bg: u32,
    pub active_fg: u32,
    /// 成功、错误、警告提示文字。
    pub ok_text: u32,
    pub warn_text: u32,
}

pub const DARK_PALETTE: Palette = Palette {
    surface: 0x20252c,
    border: 0x39424e,
    text: 0xe6edf3,
    muted: 0x9aa7b4,
    hover: 0x2b323c,
    active_bg: 0x263a57,
    active_fg: 0x93c5fd,
    ok_text: 0x86efac,
    warn_text: 0xfbbf24,
};

pub const LIGHT_PALETTE: Palette = Palette {
    surface: 0xffffff,
    border: 0xdfe3e8,
    text: 0x24292f,
    muted: 0x6b7280,
    hover: 0xf1f3f5,
    active_bg: 0xe8f0fe,
    active_fg: 0x2563eb,
    ok_text: 0x15803d,
    warn_text: 0xb45309,
};

/// 剪贴板历史窗口的色值。
pub struct ClipPalette {
    pub bg: u32,
    pub text: u32,
    pub muted: u32,
    pub border: u32,
    pub card: u32,
    pub card_hover: u32,
    pub actions: u32,
    pub press: u32,
    pub selected: u32,
    pub selected_border: u32,
    pub accent: u32,
    pub notice: u32,
    pub notice_text: u32,
    pub danger: u32,
    pub danger_bg: u32,
    pub success: u32,
    /// 右键菜单与弹出卡片（`.history-context-menu`、`.confirm-dialog`）。
    pub menu_bg: u32,
    pub menu_border: u32,
    pub menu_hover_bg: u32,
    pub menu_hover_fg: u32,
    pub dialog_text: u32,
    pub dialog_muted: u32,
    /// 勾选框（control-theme.css）。
    pub check_fill: u32,
    pub control_border: u32,
}

pub const CLIP_LIGHT: ClipPalette = ClipPalette {
    bg: 0xffffff,
    text: 0x24292f,
    muted: 0x6b7280,
    border: 0xe1e4e8,
    card: 0xf6f7f8,
    card_hover: 0xeef1f4,
    actions: 0xeef1f4,
    press: 0xdfe5ec,
    selected: 0xeff6ff,
    selected_border: 0x93b4ff,
    accent: 0x2563eb,
    notice: 0xeef4fc,
    notice_text: 0x315a86,
    danger: 0xb42318,
    danger_bg: 0xfff1f0,
    success: 0x24834b,
    menu_bg: 0xffffff,
    menu_border: 0xe4e4e7,
    menu_hover_bg: 0xeff6ff,
    menu_hover_fg: 0x2563eb,
    dialog_text: 0x24292f,
    dialog_muted: 0x57606a,
    check_fill: 0xedf8f1,
    control_border: 0x8c959f,
};

pub const CLIP_DARK: ClipPalette = ClipPalette {
    bg: 0x1b1f24,
    text: 0xe6edf3,
    muted: 0x9aa7b4,
    border: 0x39424e,
    card: 0x252b33,
    card_hover: 0x2d3540,
    actions: 0x2d3540,
    press: 0x3a4452,
    selected: 0x223c62,
    selected_border: 0x6386b8,
    accent: 0x6ea8ff,
    notice: 0x22364d,
    notice_text: 0xc4dfff,
    danger: 0xff9c95,
    danger_bg: 0x3b2327,
    success: 0x73d69a,
    menu_bg: 0x20252c,
    menu_border: 0x39424e,
    menu_hover_bg: 0x223c62,
    menu_hover_fg: 0xa9ccff,
    dialog_text: 0xe6edf3,
    dialog_muted: 0xa8b3c2,
    check_fill: 0x203d2c,
    control_border: 0x768390,
};

/// 设置主窗口色值（半透明的 soft/hover 已按所在底色混合）。
pub struct DeskPalette {
    pub bg: u32,
    pub surface: u32,
    pub nav: u32,
    pub border: u32,
    pub line: u32,
    pub sep: u32,
    pub text: u32,
    pub muted: u32,
    pub accent: u32,
    pub accent_fg: u32,
    /// 强调色淡底（导航选中、分段控件选中、工具卡片图标底）。
    pub soft: u32,
    /// 悬停底色：在导航底色上 / 在卡片底色上。
    pub hover_nav: u32,
    pub hover: u32,
    pub danger: u32,
    pub success: u32,
    /// 提示条（`.notice`：强调色 8% 混合卡片底色）。
    pub notice: u32,
    pub notice_error: u32,
}

pub const DESK_LIGHT: DeskPalette = DeskPalette {
    bg: 0xf9fafb,
    surface: 0xffffff,
    nav: 0xf3f4f6,
    border: 0xe3e6ea,
    line: 0xd3d8de,
    sep: 0xeceef1,
    text: 0x1f2328,
    muted: 0x5f6670,
    accent: 0x3b5bdb,
    accent_fg: 0x3b5bdb,
    soft: 0xebeffb,
    hover_nav: 0xe5e6e8,
    hover: 0xf0f0f0,
    danger: 0xcf222e,
    success: 0x24834b,
    notice: 0xf3f5fd,
    notice_error: 0xfaebec,
};

pub const DESK_DARK: DeskPalette = DeskPalette {
    bg: 0x202227,
    surface: 0x26292e,
    nav: 0x1b1d21,
    border: 0x33373e,
    line: 0x454a52,
    sep: 0x30343a,
    text: 0xe8eaed,
    muted: 0x9aa0a8,
    accent: 0x4c6ef5,
    accent_fg: 0x9db0ff,
    soft: 0x2c3558,
    hover_nav: 0x2d2f33,
    hover: 0x373a3f,
    danger: 0xff7b72,
    success: 0x73d69a,
    notice: 0x2a3040,
    notice_error: 0x3a2b2d,
};

/// 设置主窗口的当前色值。
pub fn desk() -> &'static DeskPalette {
    if dark() {
        &DESK_DARK
    } else {
        &DESK_LIGHT
    }
}

/// 当前主题是否为深色。
pub fn dark() -> bool {
    DARK.load(Ordering::Relaxed)
}

/// 剪贴板历史窗口的当前色值。
pub fn clip() -> &'static ClipPalette {
    if dark() {
        &CLIP_DARK
    } else {
        &CLIP_LIGHT
    }
}

/// 当前调色板。
pub fn c() -> &'static Palette {
    if DARK.load(Ordering::Relaxed) {
        &DARK_PALETTE
    } else {
        &LIGHT_PALETTE
    }
}

/// 主题设置与系统外观 → 是否深色。
pub fn is_dark(theme: &Theme, system: WindowAppearance) -> bool {
    match theme {
        Theme::Dark => true,
        Theme::Light => false,
        Theme::System => matches!(
            system,
            WindowAppearance::Dark | WindowAppearance::VibrantDark
        ),
    }
}

/// 按当前设置与系统外观刷新；返回主题是否改变（调用方据此重绘窗口）。
/// 同时让 gpui-component 的组件（输入框、文本域）使用相同的浅色/深色模式。
pub fn refresh(cx: &mut App) -> bool {
    let theme = crate::shell::settings(cx).appearance.theme;
    let dark = is_dark(&theme, cx.window_appearance());
    let changed = DARK.swap(dark, Ordering::Relaxed) != dark;
    if changed || !SYNCED.swap(true, Ordering::Relaxed) {
        ComponentTheme::change(
            if dark {
                ThemeMode::Dark
            } else {
                ThemeMode::Light
            },
            None,
            cx,
        );
    }
    changed
}

#[cfg(test)]
mod tests {
    use super::{is_dark, DARK_PALETTE, LIGHT_PALETTE};
    use gpui_kit::WindowAppearance;
    use snap_core::settings::Theme;

    #[test]
    fn system_theme_follows_the_os_appearance() {
        assert!(is_dark(&Theme::System, WindowAppearance::Dark));
        assert!(is_dark(&Theme::System, WindowAppearance::VibrantDark));
        assert!(!is_dark(&Theme::System, WindowAppearance::Light));
        assert!(is_dark(&Theme::Dark, WindowAppearance::Light));
        assert!(!is_dark(&Theme::Light, WindowAppearance::Dark));
        assert_ne!(DARK_PALETTE.text, LIGHT_PALETTE.text);
    }
}
