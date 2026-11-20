//! 贴图的全局外观配置；窗口、像素缓存与变换仍在各前端外壳。
use serde::{Deserialize, Serialize};

/// 全局贴图外观只影响显示；阴影由 WebView 按尺寸和焦点计算，原图像素保持独立。
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields, default)]
pub struct PinAppearance {
    pub shadow: PinShadow,
    pub corner_radius: u8,
    /// 只接受 #RRGGBB，旧配置缺少字段时沿用默认中性灰。
    pub border_color: String,
}
#[derive(Clone, Debug, Default, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum PinShadow {
    #[default]
    Auto,
    Soft,
    None,
}
impl Default for PinAppearance {
    fn default() -> Self {
        Self {
            shadow: PinShadow::Auto,
            corner_radius: 8,
            border_color: "#a1a1aa".into(),
        }
    }
}
impl PinAppearance {
    pub fn validate(&self) -> Result<(), String> {
        let color = self.border_color.as_bytes();
        if color.len() != 7 || color[0] != b'#' || !color[1..].iter().all(u8::is_ascii_hexdigit) {
            return Err("边框颜色需为六位十六进制颜色。".into());
        }
        if self.corner_radius > 24 {
            return Err("贴图圆角需为 0～24 像素。".into());
        }
        Ok(())
    }
}
