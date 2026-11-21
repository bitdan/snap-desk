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
    /// 设置窗口页面底色与导航底色。
    pub page: u32,
    pub nav: u32,
    pub accent: u32,
    pub accent_soft: u32,
    pub danger: u32,
    pub success: u32,
    /// 关闭状态的开关轨道。
    pub toggle_off: u32,
    /// 列表类窗口（剪贴板历史、预览、结果、贴图管理）的窗口底色、面板底色与内嵌区底色。
    pub window_bg: u32,
    pub panel: u32,
    pub inset: u32,
    /// 成功、错误、警告提示文字。
    pub ok_text: u32,
    pub error_text: u32,
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
    page: 0x161a1f,
    nav: 0x1b2026,
    accent: 0x3b82f6,
    accent_soft: 0x1e3a5f,
    danger: 0xf87171,
    success: 0x4ade80,
    toggle_off: 0x4b5563,
    window_bg: 0x0f1115,
    panel: 0x181b20,
    inset: 0x14171c,
    ok_text: 0x86efac,
    error_text: 0xfb7185,
    warn_text: 0xfbbf24,
};

pub const LIGHT_PALETTE: Palette = Palette {
    surface: 0xffffff,
    border: 0xd4d4d8,
    text: 0x27272a,
    muted: 0x71717a,
    hover: 0xf4f4f5,
    active_bg: 0xdbeafe,
    active_fg: 0x1d4ed8,
    page: 0xf4f4f5,
    nav: 0xfafafa,
    accent: 0x2563eb,
    accent_soft: 0xeff6ff,
    danger: 0xb91c1c,
    success: 0x16a34a,
    toggle_off: 0xd4d4d8,
    window_bg: 0xf4f4f5,
    panel: 0xfafafa,
    inset: 0xececef,
    ok_text: 0x15803d,
    error_text: 0xb91c1c,
    warn_text: 0xb45309,
};

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
