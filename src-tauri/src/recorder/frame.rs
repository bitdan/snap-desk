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
pub struct AnnotationLayer(image::RgbaImage);
impl AnnotationLayer {
    pub fn new(
        shapes: &[crate::annotations::Annotation],
        width: u32,
        height: u32,
        origin: crate::annotations::Point,
    ) -> Result<Option<Self>, String> {
        if shapes.is_empty() {
            return Ok(None);
        }
        let mut layer = image::RgbaImage::new(width, height);
        crate::annotations::render(&mut layer, shapes, origin)?;
        Ok(Some(Self(layer)))
    }

    /// 标注为 RGBA，采集帧为 BGRA；按通道交换后做 alpha 混合，保留视频不透明语义。
    pub fn apply(&self, image: &mut BgraImage) -> Result<(), String> {
        if image.dimensions() != self.0.dimensions() {
            return Err("录屏尺寸已变化，请重新选择标注范围。".into());
        }
        for (target, source) in image.data.chunks_exact_mut(4).zip(self.0.pixels()) {
            let alpha = u32::from(source[3]);
            if alpha == 0 {
                continue;
            }
            for (channel, source_channel) in [(0, 2), (1, 1), (2, 0)] {
                target[channel] = ((u32::from(source[source_channel]) * alpha
                    + u32::from(target[channel]) * (255 - alpha)
                    + 127)
                    / 255) as u8;
            }
            target[3] = 255;
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
        AnnotationLayer(layer).apply(&mut frame).unwrap();
        assert_eq!(frame.data, vec![35, 70, 140, 255, 1, 2, 3, 255]);
    }

    #[test]
    fn empty_annotations_skip_layer_allocation_and_size_changes_are_rejected() {
        assert!(
            AnnotationLayer::new(&[], 2, 2, crate::annotations::Point { x: 0.0, y: 0.0 })
                .unwrap()
                .is_none()
        );
        assert!(AnnotationLayer(image::RgbaImage::new(2, 2))
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
    pub annotations: Option<std::sync::Arc<super::live_annotations::Layer>>,
    pub generation: u64,
    pub pts: i64,
    pub image: BgraImage,
}
