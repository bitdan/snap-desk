//! 截图选区的尺寸偏好：比例锁定、尺寸条的显示单位与尺寸预设。
//!
//! 截图浮层的“位置与尺寸”条读写这些偏好，修改即保存为配置目录下的 `capture-size.json`
//! （`{ "version": 1, "sizing": … }`）；文件缺失、损坏或版本不符时用默认值，不影响截图。
//! 预设是导出图片的物理像素，与显示器缩放无关；单位只决定尺寸条里显示和输入的数值
//! （dp = 物理像素 ÷ 选区所在屏的缩放）。
use serde::{Deserialize, Serialize};
use std::path::{Path, PathBuf};

/// 预设最多保存的条数；尺寸条一行放得下。
pub const MAX_PRESETS: usize = 12;
/// 单边上限：远大于任何显示器组合，只用来挡住损坏的文件。
const MAX_SIDE: u32 = 65_535;
/// 尺寸条提供的常用比例（宽:高）。
pub const RATIOS: [Size; 6] = [
    Size::new(1, 1),
    Size::new(4, 3),
    Size::new(3, 4),
    Size::new(16, 9),
    Size::new(9, 16),
    Size::new(3, 2),
];

/// 宽与高；既表示预设尺寸（物理像素），也表示比例（宽:高，已约分）。
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Size {
    pub width: u32,
    pub height: u32,
}

impl Size {
    pub const fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    fn valid(self) -> bool {
        (1..=MAX_SIDE).contains(&self.width) && (1..=MAX_SIDE).contains(&self.height)
    }

    /// 约分后的比例，例如 1080×1440 → 3:4。
    pub fn ratio(self) -> Self {
        let divisor = gcd(self.width, self.height).max(1);
        Self::new(self.width / divisor, self.height / divisor)
    }
}

fn gcd(a: u32, b: u32) -> u32 {
    if b == 0 { a } else { gcd(b, a % b) }
}

/// 尺寸条显示与输入的单位。
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Unit {
    /// 物理像素，即导出图片的尺寸。
    #[default]
    Px,
    /// 按显示器缩放换算的逻辑像素。
    Dp,
}

#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub struct Sizing {
    /// 锁定的宽高比（已约分）；None 为自由比例。锁定时拖动新建与缩放选区都保持该比例。
    pub ratio: Option<Size>,
    pub unit: Unit,
    /// 尺寸预设，按添加顺序排列。
    pub presets: Vec<Size>,
}

impl Default for Sizing {
    fn default() -> Self {
        Self {
            ratio: None,
            unit: Unit::Px,
            // 常见输出：横屏视频、方图、3:4 竖图（如小红书）、9:16 竖屏。
            presets: vec![
                Size::new(1920, 1080),
                Size::new(1280, 720),
                Size::new(1080, 1080),
                Size::new(1080, 1440),
                Size::new(1080, 1920),
            ],
        }
    }
}

impl Sizing {
    /// 外部修改或旧文件可能带来非法值：丢弃无效与重复的预设，超出上限的截断，比例重新约分。
    fn normalized(mut self) -> Self {
        let mut presets: Vec<Size> = Vec::new();
        for size in self.presets {
            if size.valid() && !presets.contains(&size) && presets.len() < MAX_PRESETS {
                presets.push(size);
            }
        }
        self.presets = presets;
        self.ratio = self.ratio.filter(|ratio| ratio.valid()).map(Size::ratio);
        self
    }

    /// 存为预设；无效、已存在或已满时返回 false。
    pub fn add_preset(&mut self, size: Size) -> bool {
        if !size.valid() || self.presets.contains(&size) || self.presets.len() >= MAX_PRESETS {
            return false;
        }
        self.presets.push(size);
        true
    }

    pub fn remove_preset(&mut self, size: Size) -> bool {
        let before = self.presets.len();
        self.presets.retain(|existing| *existing != size);
        self.presets.len() != before
    }

    /// 锁定比例（传入任意宽高，存为约分后的比例）；None 解除锁定。
    pub fn set_ratio(&mut self, ratio: Option<Size>) {
        self.ratio = ratio.filter(|ratio| ratio.valid()).map(Size::ratio);
    }
}

#[derive(Serialize, Deserialize)]
struct Stored {
    version: u32,
    sizing: Sizing,
}

pub fn default_path() -> Option<PathBuf> {
    Some(crate::paths::config_dir()?.join("capture-size.json"))
}

pub fn load(path: &Path) -> Sizing {
    std::fs::read(path)
        .ok()
        .and_then(|bytes| serde_json::from_slice::<Stored>(&bytes).ok())
        .filter(|stored| stored.version == 1)
        .map(|stored| stored.sizing.normalized())
        .unwrap_or_default()
}

/// 写入失败只影响下次打开截图时的偏好，调用方记录日志即可。
pub fn save(path: &Path, sizing: &Sizing) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let bytes = serde_json::to_vec_pretty(&Stored { version: 1, sizing: sizing.clone() })
        .map_err(|e| e.to_string())?;
    std::fs::write(path, bytes).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn saves_and_loads_ratio_unit_and_presets() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("nested").join("capture-size.json");
        assert_eq!(load(&path), Sizing::default());
        let mut sizing = Sizing::default();
        sizing.set_ratio(Some(Size::new(1080, 1440)));
        sizing.unit = Unit::Dp;
        assert!(sizing.add_preset(Size::new(900, 383)));
        assert!(!sizing.add_preset(Size::new(900, 383)));
        assert!(sizing.remove_preset(Size::new(1920, 1080)));
        save(&path, &sizing).unwrap();
        let loaded = load(&path);
        assert_eq!(loaded.ratio, Some(Size::new(3, 4)));
        assert_eq!(loaded, sizing);
    }

    #[test]
    fn drops_invalid_duplicate_and_excess_presets_and_ignores_corrupt_files() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("capture-size.json");
        let mut presets: Vec<_> = (1..=20).map(|i| serde_json::json!({ "width": i * 100, "height": 100 })).collect();
        presets.insert(0, serde_json::json!({ "width": 0, "height": 100 }));
        presets.insert(1, serde_json::json!({ "width": 100, "height": 100 }));
        let stored = serde_json::json!({
            "version": 1,
            "sizing": { "ratio": { "width": 0, "height": 9 }, "unit": "px", "presets": presets }
        });
        std::fs::write(&path, serde_json::to_vec(&stored).unwrap()).unwrap();
        let loaded = load(&path);
        assert_eq!(loaded.ratio, None);
        assert_eq!(loaded.presets.len(), MAX_PRESETS);
        assert_eq!(loaded.presets[0], Size::new(100, 100));
        assert_eq!(loaded.presets[1], Size::new(200, 100));
        std::fs::write(&path, b"broken").unwrap();
        assert_eq!(load(&path), Sizing::default());
        std::fs::write(&path, br#"{"version":2,"sizing":{}}"#).unwrap();
        assert_eq!(load(&path), Sizing::default());
    }

    #[test]
    fn presets_stop_at_the_limit() {
        let mut sizing = Sizing { presets: Vec::new(), ..Sizing::default() };
        for i in 1..=MAX_PRESETS as u32 {
            assert!(sizing.add_preset(Size::new(i, i)));
        }
        assert!(!sizing.add_preset(Size::new(999, 999)));
        assert!(!sizing.add_preset(Size::new(0, 5)));
    }
}
