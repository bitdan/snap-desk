//! GIF 编码复用录屏队列和去除暂停后的时间戳，只缓存一帧，不积攒整段录像。
use super::{
    encoder::{Encoder, EncoderKind},
    frame::BgraImage,
};
use std::{
    fs::File,
    io::{BufWriter, Write},
    path::Path,
};

/// 编码线程独占文件和待写帧；finish 成功后交给管理器同步并发布文件。
pub struct GifEncoder {
    writer: Option<gif::Encoder<BufWriter<File>>>,
    pending: Option<gif::Frame<'static>>,
    width: u32,
    height: u32,
    /// 已输出的百分之一秒数；按绝对时间量化，避免逐帧舍入误差累积。
    written: i64,
    last_pts: Option<i64>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn preserves_colors_timing_and_long_static_frames() {
        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("test.gif");
        let mut encoder = GifEncoder::new(&path, 2, 2).unwrap();
        let mut image = BgraImage::new(2, 2);
        image.data = [0, 0, 255, 0].repeat(4);
        encoder.push(&image, 0).unwrap();
        image.data = [255, 0, 0, 0].repeat(4);
        encoder.push(&image, 5_000_000).unwrap();
        encoder.finish(7_000_000_000).unwrap();
        let mut options = gif::DecodeOptions::new();
        options.set_color_output(gif::ColorOutput::RGBA);
        let mut decoder = options.read_info(File::open(path).unwrap()).unwrap();
        let first = decoder.read_next_frame().unwrap().unwrap();
        assert_eq!(first.delay, 50);
        assert_eq!(&first.buffer[..4], &[255, 0, 0, 255]);
        let mut total = u64::from(first.delay);
        while let Some(frame) = decoder.read_next_frame().unwrap() {
            assert_eq!(&frame.buffer[..4], &[0, 0, 255, 255]);
            total += u64::from(frame.delay);
        }
        assert_eq!(total, 70_000);
    }

    #[test]
    fn rejects_empty_invalid_and_out_of_order_frames() {
        let root = tempfile::tempdir().unwrap();
        let mut encoder = GifEncoder::new(&root.path().join("test.gif"), 2, 2).unwrap();
        assert!(encoder.finish(0).is_err());
        assert!(encoder.push(&BgraImage::new(3, 2), 0).is_err());
        let mut image = BgraImage::new(2, 2);
        image.data.pop();
        assert!(encoder.push(&image, 0).is_err());
        encoder.push(&BgraImage::new(2, 2), 0).unwrap();
        assert!(encoder.push(&BgraImage::new(2, 2), 0).is_err());
        encoder.finish(1_000_000).unwrap();
        assert!(encoder.push(&BgraImage::new(2, 2), 2_000_000).is_err());
    }
}

impl GifEncoder {
    pub fn new(path: &Path, width: u32, height: u32) -> Result<Self, String> {
        let w = u16::try_from(width).map_err(|e| e.to_string())?;
        let h = u16::try_from(height).map_err(|e| e.to_string())?;
        if w == 0 || h == 0 {
            return Err("GIF 尺寸不能为空。".into());
        }
        let file = File::create(path).map_err(|e| e.to_string())?;
        let mut writer =
            gif::Encoder::new(BufWriter::new(file), w, h, &[]).map_err(|e| e.to_string())?;
        writer
            .set_repeat(gif::Repeat::Infinite)
            .map_err(|e| e.to_string())?;
        Ok(Self {
            writer: Some(writer),
            pending: None,
            width,
            height,
            written: 0,
            last_pts: None,
        })
    }

    /// 下一帧到来才知道上一帧展示多久；长时间静止拆为多帧，避免 u16 延迟溢出。
    fn flush_until(&mut self, end: i64) -> Result<(), String> {
        let Some(frame) = self.pending.as_mut() else {
            return Ok(());
        };
        let target = ((end.max(0) + 50_000) / 100_000).max(self.written + 2);
        let writer = self.writer.as_mut().ok_or("GIF 编码已结束。")?;
        while self.written < target {
            let mut delay = (target - self.written).min(i64::from(u16::MAX));
            if target - self.written - delay == 1 {
                delay -= 1;
            }
            frame.delay = delay as u16;
            writer.write_frame(frame).map_err(|e| e.to_string())?;
            self.written += delay;
        }
        Ok(())
    }
}

impl Encoder for GifEncoder {
    fn kind(&self) -> EncoderKind {
        EncoderKind::Software
    }
    // 此统计字段专指 NV12，GIF 不进行 NV12 转换。
    fn converted_bytes(&self) -> u64 {
        0
    }
    fn push(&mut self, image: &BgraImage, pts: i64) -> Result<(), String> {
        if self.writer.is_none()
            || image.dimensions() != (self.width, self.height)
            || self.last_pts.is_some_and(|last| pts <= last)
            || pts < 0
        {
            return Err("GIF 帧尺寸、时间戳或编码状态无效。".into());
        }
        let mut rgba = image.opaque_rgba()?;
        self.flush_until(pts)?;
        let mut frame =
            gif::Frame::from_rgba_speed(self.width as u16, self.height as u16, &mut rgba, 10);
        frame.dispose = gif::DisposalMethod::Keep;
        self.pending = Some(frame);
        self.last_pts = Some(pts);
        Ok(())
    }
    fn finish(&mut self, end: i64) -> Result<(), String> {
        if self.pending.is_none() {
            return Err("尚未捕获任何 GIF 帧。".into());
        }
        self.flush_until(end)?;
        // 显式写入结束标记并 flush，写盘错误交给流水线清理临时输出。
        let mut file = self
            .writer
            .take()
            .ok_or("GIF 编码已结束。")?
            .into_inner()
            .map_err(|e| e.to_string())?;
        file.flush().map_err(|e| e.to_string())?;
        self.pending = None;
        Ok(())
    }
}
