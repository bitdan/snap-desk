//! 视频帧始终留在 Rust。CPU 数据独立拥有内存，不借用 WGC 帧池表面。
/// 紧密排列的 BGRA 像素；独立类型防止误传给按 RGBA 解释通道的截图 API。
/// 采集端只去掉 D3D 行尾对齐；视频编码转 NV12，GIF 编码转不透明 RGBA。
#[derive(Clone)]
pub struct BgraImage {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
}

/// 准备阶段的标注以输出像素绘成透明层；无标注时不增加每帧处理开销。
pub struct AnnotationLayer {
    pixels: image::RgbaImage,
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}
impl AnnotationLayer {
    pub fn new(
        shapes: &[crate::shared::annotations::Annotation],
        width: u32,
        height: u32,
        origin: crate::shared::annotations::Point,
    ) -> Result<Option<Self>, String> {
        if shapes.is_empty() {
            return Ok(None);
        }
        // 普通图形只在其边界内栅格化；避免长录屏后导出时为每个箭头扫描整屏。
        let (left, top, right, bottom) = if shapes
            .iter()
            .any(|s| matches!(s.kind, crate::shared::annotations::Kind::Spotlight))
        {
            (0, 0, width, height)
        } else {
            let (mut left, mut top, mut right, mut bottom) =
                (f64::from(width), f64::from(height), 0.0f64, 0.0f64);
            for shape in shapes {
                // 包含箭头翼和抗锯齿边缘；PNG 字形也使用同一输出坐标空间。
                let margin = shape.width * 8.0 + 2.0;
                for point in std::iter::once(&shape.start)
                    .chain(std::iter::once(&shape.end))
                    .chain(shape.points.iter())
                {
                    left = left.min(point.x - origin.x - margin);
                    top = top.min(point.y - origin.y - margin);
                    right = right.max(point.x - origin.x + margin);
                    bottom = bottom.max(point.y - origin.y + margin);
                }
            }
            (
                left.floor().clamp(0.0, f64::from(width)) as u32,
                top.floor().clamp(0.0, f64::from(height)) as u32,
                right.ceil().clamp(0.0, f64::from(width)) as u32,
                bottom.ceil().clamp(0.0, f64::from(height)) as u32,
            )
        };
        if right <= left || bottom <= top {
            return Ok(None);
        }
        let mut layer = image::RgbaImage::new(right - left, bottom - top);
        crate::shared::annotations::render(
            &mut layer,
            shapes,
            crate::shared::annotations::Point {
                x: origin.x + f64::from(left),
                y: origin.y + f64::from(top),
            },
        )?;
        Ok(Self::cropped(layer).map(|mut layer| {
            layer.x += left;
            layer.y += top;
            layer.width = width;
            layer.height = height;
            layer
        }))
    }

    /// 缓存只保留非透明边界，普通箭头不长期占用整屏 RGBA 内存。
    fn cropped(layer: image::RgbaImage) -> Option<Self> {
        let (width, height) = layer.dimensions();
        let (mut left, mut top, mut right, mut bottom) = (width, height, 0, 0);
        for (x, y, p) in layer.enumerate_pixels() {
            if p[3] > 0 {
                left = left.min(x);
                top = top.min(y);
                right = right.max(x + 1);
                bottom = bottom.max(y + 1);
            }
        }
        if left >= right || top >= bottom {
            return None;
        }
        Some(Self {
            pixels: image::imageops::crop_imm(&layer, left, top, right - left, bottom - top)
                .to_image(),
            x: left,
            y: top,
            width,
            height,
        })
    }
    /// 标注为 RGBA，采集帧为 BGRA；按通道交换后做 alpha 混合，保留视频不透明语义。
    pub fn apply(&self, image: &mut BgraImage) -> Result<(), String> {
        self.apply_opacity(image, 1.0)
    }
    /// 淡出只改变混合权重，不重新栅格化文字或箭头。
    pub fn apply_opacity(&self, image: &mut BgraImage, opacity: f64) -> Result<(), String> {
        if image.dimensions() != (self.width, self.height) {
            return Err("录屏尺寸已变化，请重新选择标注范围。".into());
        }
        for (x, y, p) in self.pixels.enumerate_pixels() {
            let alpha = (f64::from(p[3]) * opacity.clamp(0.0, 1.0)).round() as u32;
            if alpha == 0 {
                continue;
            }
            let i = (((self.y + y) * self.width + self.x + x) * 4) as usize;
            for (c, sc) in [(0, 2), (1, 1), (2, 0)] {
                image.data[i + c] = ((u32::from(p[sc]) * alpha
                    + u32::from(image.data[i + c]) * (255 - alpha)
                    + 127)
                    / 255) as u8;
            }
            image.data[i + 3] = 255;
        }
        Ok(())
    }
}

#[cfg(test)]
mod annotation_layer_tests {
    use super::*;

    #[test]
    fn mixes_rgba_layer_into_bgra_without_changing_transparent_pixels() {
        let mut layer = image::RgbaImage::new(2, 1);
        layer.put_pixel(0, 0, image::Rgba([200, 100, 50, 128]));
        let mut frame = BgraImage {
            width: 2,
            height: 1,
            data: vec![20, 40, 80, 255, 1, 2, 3, 255],
        };
        AnnotationLayer::cropped(layer)
            .unwrap()
            .apply(&mut frame)
            .unwrap();
        assert_eq!(frame.data, vec![35, 70, 140, 255, 1, 2, 3, 255]);
    }

    #[test]
    fn empty_annotations_skip_layer_allocation_and_size_changes_are_rejected() {
        assert!(AnnotationLayer::new(
            &[],
            2,
            2,
            crate::shared::annotations::Point { x: 0.0, y: 0.0 }
        )
        .unwrap()
        .is_none());
        assert!(AnnotationLayer {
            pixels: image::RgbaImage::new(2, 2),
            x: 0,
            y: 0,
            width: 2,
            height: 2
        }
        .apply(&mut BgraImage::new(3, 2))
        .is_err());
    }
}

impl BgraImage {
    pub fn new(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            data: vec![0; width as usize * height as usize * 4],
        }
    }

    pub fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    /// GIF 量化需要 RGBA；WGC 的 alpha 不作为透明度，整张屏幕帧保持不透明。
    pub fn opaque_rgba(&self) -> Result<Vec<u8>, String> {
        let size = (self.width as usize)
            .checked_mul(self.height as usize)
            .and_then(|size| size.checked_mul(4))
            .ok_or("帧尺寸溢出。")?;
        if self.data.len() != size {
            return Err("BGRA 帧长度无效。".into());
        }
        let mut rgba = self.data.clone();
        for pixel in rgba.chunks_exact_mut(4) {
            pixel.swap(0, 2);
            pixel[3] = 255;
        }
        Ok(rgba)
    }
}

pub struct VideoFrame {
    /// WGC SystemRelativeTime，单位 100 ns，未经暂停时间轴转换。
    pub source_time: i64,
    pub image: BgraImage,
}

/// 队列只携带已投影的时间与代次，编码线程可丢弃暂停前的排队帧。
pub struct QueuedFrame {
    /// 入队时固定快照，排队帧不能套用未来的标注版本。
    pub annotations: Option<std::sync::Arc<super::annotation_layer::Layer>>,
    pub generation: u64,
    pub pts: i64,
    pub image: BgraImage,
}
