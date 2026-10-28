//! 标注合成计划：缓存静态栅格和淡出权重，动态滤镜仅从干净帧局部取样。
use super::{annotation_track::TimedAnnotation, frame};
use crate::shared::annotations::{self, Annotation, Point};
use std::sync::{Arc, Mutex};
/// 普通图形只在修改时绘成透明层；动态滤镜才从每一帧的干净像素重新计算。
pub enum Layer {
    Mixed(Vec<Paint>),
    TimedStatic(Vec<(TimedAnnotation, Option<frame::AnnotationLayer>)>),
    Static(Option<frame::AnnotationLayer>),
    Dynamic(Vec<Annotation>),
    Timed {
        tracks: Vec<TimedAnnotation>,
        width: u32,
        height: u32,
        cache: Mutex<Option<(Vec<u8>, Arc<Layer>)>>,
    },
}
/// 静态连续图层只栅格化一次；动态滤镜只读取其矩形区域的干净 BGRA 像素。
pub enum Paint {
    Static(frame::AnnotationLayer),
    Filter(Annotation),
}
impl Layer {
    /// 无时间轴的构造入口仅供合成回归测试使用；实际录屏统一使用 timed。
    #[cfg(test)]
    pub(super) fn new(shapes: &[Annotation], width: u32, height: u32) -> Result<Self, String> {
        annotations::validate_live(shapes, width, height)?;
        Self::compiled(shapes, width, height)
    }
    fn compiled(shapes: &[Annotation], width: u32, height: u32) -> Result<Self, String> {
        if shapes.iter().any(annotations::dynamic_filter) {
            if shapes
                .iter()
                .filter(|s| annotations::dynamic_filter(s))
                .count()
                > 8
            {
                return Err("录制中最多支持 8 个实时滤镜。".into());
            }
            // 橡皮擦和聚光灯需要跨图层掩码，保留原有合成路径保证顺序与亮区语义。
            if shapes.iter().any(|s| {
                matches!(
                    s.kind,
                    annotations::Kind::Erase | annotations::Kind::Spotlight
                )
            }) {
                return Ok(Self::Dynamic(shapes.to_vec()));
            }
            let mut paints = Vec::new();
            let mut start = 0;
            for (index, shape) in shapes.iter().enumerate() {
                if annotations::dynamic_filter(shape) {
                    if let Some(layer) = frame::AnnotationLayer::new(
                        &shapes[start..index],
                        width,
                        height,
                        Point { x: 0.0, y: 0.0 },
                    )? {
                        paints.push(Paint::Static(layer));
                    }
                    paints.push(Paint::Filter(shape.clone()));
                    start = index + 1;
                }
            }
            if let Some(layer) = frame::AnnotationLayer::new(
                &shapes[start..],
                width,
                height,
                Point { x: 0.0, y: 0.0 },
            )? {
                paints.push(Paint::Static(layer));
            }
            Ok(Self::Mixed(paints))
        } else {
            Ok(Self::Static(frame::AnnotationLayer::new(
                shapes,
                width,
                height,
                Point { x: 0.0, y: 0.0 },
            )?))
        }
    }
    pub(super) fn timed(
        tracks: Vec<TimedAnnotation>,
        width: u32,
        height: u32,
    ) -> Result<Self, String> {
        if tracks.iter().all(|track| {
            !annotations::dynamic_filter(&track.shape)
                && !matches!(
                    track.shape.kind,
                    annotations::Kind::Erase | annotations::Kind::Spotlight
                )
        }) {
            return Ok(Self::TimedStatic(
                tracks
                    .into_iter()
                    .map(|track| {
                        let layer = frame::AnnotationLayer::new(
                            std::slice::from_ref(&track.shape),
                            width,
                            height,
                            Point { x: 0.0, y: 0.0 },
                        )?;
                        Ok((track, layer))
                    })
                    .collect::<Result<Vec<_>, String>>()?,
            ));
        }
        Ok(Self::Timed {
            tracks,
            width,
            height,
            cache: Mutex::new(None),
        })
    }
    pub fn apply(&self, frame: &mut frame::BgraImage, pts: i64) -> Result<(), String> {
        match self {
            Self::TimedStatic(tracks) => {
                for (track, layer) in tracks {
                    let alpha = track.alpha(pts.max(0) as u64 / 10_000);
                    if let Some(layer) = layer.as_ref().filter(|_| alpha > 0.0) {
                        layer.apply_opacity(frame, alpha)?;
                    }
                }
                Ok(())
            }
            Self::Mixed(paints) => {
                // 先对所有滤镜取样，再绘制任何图层，后面的滤镜不能读到之前画入的箭头。
                let patches = paints
                    .iter()
                    .map(|paint| -> Result<Option<image::RgbaImage>, String> {
                        let Paint::Filter(shape) = paint else {
                            return Ok(None);
                        };
                        let x = (shape.start.x.floor().max(0.0) as u32).min(frame.width - 1);
                        let y = (shape.start.y.floor().max(0.0) as u32).min(frame.height - 1);
                        let w = (shape.end.x.ceil() as u32)
                            .min(frame.width)
                            .saturating_sub(x)
                            .max(1);
                        let h = (shape.end.y.ceil() as u32)
                            .min(frame.height)
                            .saturating_sub(y)
                            .max(1);
                        let mut crop = image::RgbaImage::new(w, h);
                        for cy in 0..h {
                            for cx in 0..w {
                                let i = (((y + cy) * frame.width + x + cx) * 4) as usize;
                                crop.put_pixel(
                                    cx,
                                    cy,
                                    image::Rgba([
                                        frame.data[i + 2],
                                        frame.data[i + 1],
                                        frame.data[i],
                                        255,
                                    ]),
                                );
                            }
                        }
                        let mut local = shape.clone();
                        local.start.x -= f64::from(x);
                        local.end.x -= f64::from(x);
                        local.start.y -= f64::from(y);
                        local.end.y -= f64::from(y);
                        Ok(Some(annotations::live_filter(&crop, &local)?))
                    })
                    .collect::<Result<Vec<_>, _>>()?;
                for (paint, patch) in paints.iter().zip(patches) {
                    match paint {
                        Paint::Static(layer) => layer.apply(frame)?,
                        Paint::Filter(shape) => {
                            let patch = patch.unwrap();
                            let left = shape.start.x;
                            let top = shape.start.y;
                            let w = (shape.end.x - left).max(1.0);
                            let h = (shape.end.y - top).max(1.0);
                            for y in top.floor().max(0.0) as u32
                                ..shape.end.y.ceil().min(f64::from(frame.height)) as u32
                            {
                                for x in left.floor().max(0.0) as u32
                                    ..shape.end.x.ceil().min(f64::from(frame.width)) as u32
                                {
                                    let sx = (((f64::from(x) + 0.5 - left) / w).clamp(0.0, 1.0)
                                        * f64::from(patch.width()))
                                        as u32;
                                    let sy = (((f64::from(y) + 0.5 - top) / h).clamp(0.0, 1.0)
                                        * f64::from(patch.height()))
                                        as u32;
                                    let p = patch.get_pixel(
                                        sx.min(patch.width() - 1),
                                        sy.min(patch.height() - 1),
                                    );
                                    let alpha = (f64::from(p[3]) * shape.opacity).round() as u32;
                                    let index = ((y * frame.width + x) * 4) as usize;
                                    for (c, sc) in [(0, 2), (1, 1), (2, 0)] {
                                        frame.data[index + c] = ((u32::from(p[sc]) * alpha
                                            + u32::from(frame.data[index + c]) * (255 - alpha)
                                            + 127)
                                            / 255)
                                            as u8;
                                    }
                                    frame.data[index + 3] = 255;
                                }
                            }
                        }
                    }
                }
                Ok(())
            }
            Self::Timed {
                tracks,
                width,
                height,
                cache,
            } => {
                let ms = pts.max(0) as u64 / 10_000;
                let key = tracks
                    .iter()
                    .map(|track| (track.alpha(ms) * 255.0).round() as u8)
                    .collect::<Vec<_>>();
                let layer = {
                    let mut cache = cache.lock().map_err(|e| e.to_string())?;
                    if cache.as_ref().is_none_or(|(old, _)| old != &key) {
                        let shapes = tracks
                            .iter()
                            .zip(&key)
                            .filter(|(_, alpha)| **alpha > 0)
                            .map(|(track, alpha)| {
                                let mut shape = track.shape.clone();
                                shape.opacity *= f64::from(*alpha) / 255.0;
                                shape
                            })
                            .collect::<Vec<_>>();
                        // 原始形状已在提交时校验；淡出的中间透明度不能再次套用输入限制。
                        *cache = Some((key, Arc::new(Self::compiled(&shapes, *width, *height)?)));
                    }
                    cache.as_ref().unwrap().1.clone()
                };
                layer.apply(frame, pts)
            }
            Self::Static(Some(layer)) => layer.apply(frame),
            Self::Static(None) => Ok(()),
            Self::Dynamic(shapes) => {
                let mut image =
                    image::RgbaImage::from_raw(frame.width, frame.height, frame.opaque_rgba()?)
                        .ok_or("录屏帧尺寸无效。")?;
                annotations::render_live(&mut image, shapes)?;
                let mut pixels = image.into_raw();
                for pixel in pixels.chunks_exact_mut(4) {
                    pixel.swap(0, 2);
                    pixel[3] = 255;
                }
                frame.data = pixels;
                Ok(())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn shape(kind: &str) -> Annotation {
        serde_json::from_value(serde_json::json!({"kind":kind,"start":{"x":2,"y":2},"end":{"x":12,"y":12},"color":"#ff0000","width":2})).unwrap()
    }
    fn frame() -> frame::BgraImage {
        frame::BgraImage {
            width: 16,
            height: 16,
            data: (0..256)
                .flat_map(|i| [i as u8, (i / 2) as u8, 64, 255])
                .collect(),
        }
    }
    #[test]
    fn local_filters_preserve_clean_sampling_and_layer_order() {
        let shapes = vec![shape("rectangle"), shape("mosaic"), shape("arrow")];
        let clean = frame();
        let mut actual = clean.clone();
        Layer::new(&shapes, 16, 16)
            .unwrap()
            .apply(&mut actual, 0)
            .unwrap();
        let mut reference =
            image::RgbaImage::from_raw(16, 16, clean.opaque_rgba().unwrap()).unwrap();
        annotations::render_live(&mut reference, &shapes).unwrap();
        let mut expected = reference.into_raw();
        for px in expected.chunks_exact_mut(4) {
            px.swap(0, 2);
        }
        assert!(actual
            .data
            .iter()
            .zip(expected)
            .all(|(a, b)| a.abs_diff(b) <= 2));
    }
    #[test]
    fn timed_cache_keeps_original_frame_clean_after_expiry() {
        let track = TimedAnnotation {
            id: "a-1".into(),
            shape: shape("arrow"),
            start_ms: 1000,
            end_ms: Some(4000),
            fade_out_ms: 500,
            order: 0,
        };
        let layer = Layer::timed(vec![track], 16, 16).unwrap();
        let mut active = frame();
        layer.apply(&mut active, 20_000_000).unwrap();
        assert_ne!(active.data, frame().data);
        let mut expired = frame();
        layer.apply(&mut expired, 40_000_000).unwrap();
        assert_eq!(expired.data, frame().data);
    }
}
