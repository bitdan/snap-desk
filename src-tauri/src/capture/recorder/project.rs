//! 录屏项目保留干净原片和独立标注轨道；重新导出始终从原片合成，不从已烧录画面擦除。
use super::{
    annotation_layer::Layer,
    annotation_track::TimedAnnotation,
    config::{EncodingPreference, OutputFormat},
    frame,
};
use serde::{Deserialize, Serialize};
use std::{
    fs,
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
};
#[cfg(windows)]
#[path = "project_windows.rs"]
mod windows;

#[derive(Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct RecordingProject {
    pub version: u32,
    pub width: u32,
    pub height: u32,
    pub fps: u32,
    pub duration_ms: u64,
    pub format: String,
    pub annotations: Vec<TimedAnnotation>,
}
pub fn source_path(video: &Path) -> PathBuf {
    video.with_file_name(format!(
        "{}-source.{}",
        video.file_stem().unwrap_or_default().to_string_lossy(),
        video.extension().unwrap_or_default().to_string_lossy()
    ))
}
fn project_path(video: &Path) -> PathBuf {
    video.with_extension("snapdesk.json")
}
impl RecordingProject {
    pub fn validate(&self) -> Result<(), String> {
        if !["mp4", "gif"].contains(&self.format.as_str())
            || self.version != 1
            || self.width == 0
            || self.height == 0
            || self.width > 16384
            || self.height > 16384
            || self.width as u64 * self.height as u64 > 67_108_864
            || ![10, 15, 20, 24, 25, 30, 50, 60].contains(&self.fps)
            || self.duration_ms == 0
            || self.duration_ms > 604_800_000
            || self.annotations.len() > 10_000
        {
            return Err("录屏项目格式或尺寸无效。".into());
        }
        let mut ids = std::collections::HashSet::new();
        let mut events = Vec::new();
        let mut raster_bytes = 0usize;
        for track in &self.annotations {
            if track.id.len() > 80
                || !ids.insert(&track.id)
                || track.start_ms >= self.duration_ms
                || track
                    .end_ms
                    .is_some_and(|end| end <= track.start_ms || end > self.duration_ms)
                || track.fade_out_ms > 1000
                || track.order > 10000
            {
                return Err("标注轨道时间或标识无效。".into());
            }
            raster_bytes += track.shape.raster.as_ref().map_or(0, String::len);
            let dynamic = i32::from(crate::shared::annotations::dynamic_filter(&track.shape));
            events.push((track.start_ms, 1, dynamic));
            events.push((track.end_ms.unwrap_or(self.duration_ms), -1, -dynamic));
            crate::shared::annotations::validate_live(
                std::slice::from_ref(&track.shape),
                self.width,
                self.height,
            )?;
        }
        if raster_bytes > 60 * 1024 * 1024 {
            return Err("项目字形数据过大。".into());
        }
        events.sort_unstable();
        let (mut count, mut filters) = (0i32, 0i32);
        for (_, delta, dynamic) in events {
            count += delta;
            filters += dynamic;
            if count > 100 || filters > 8 {
                return Err("同一时刻最多显示 100 个标注和 8 个实时滤镜。".into());
            }
        }
        Ok(())
    }
    pub fn layer(&self) -> Result<Layer, String> {
        let mut tracks = self.annotations.clone();
        tracks.sort_by_key(|t| t.order);
        Layer::timed(tracks, self.width, self.height)
    }
}
pub fn save(video: &Path, project: &RecordingProject) -> Result<(), String> {
    project.validate()?;
    let target = project_path(video);
    crate::recovery::no_links(&target)?;
    let bytes = serde_json::to_vec(project).map_err(|e| e.to_string())?;
    if bytes.len() > 64 * 1024 * 1024 {
        return Err("标注项目超过 64 MiB 限制。".into());
    }
    use std::io::Write;
    let mut pending = tempfile::NamedTempFile::new_in(video.parent().ok_or("项目目录无效。")?)
        .map_err(|e| e.to_string())?;
    pending.write_all(&bytes).map_err(|e| e.to_string())?;
    pending.as_file().sync_all().map_err(|e| e.to_string())?;
    pending.persist(target).map_err(|e| e.to_string())?;
    Ok(())
}
pub fn load(video: &Path) -> Result<Option<RecordingProject>, String> {
    let path = project_path(video);
    if !path.exists() {
        return Ok(None);
    }
    crate::recovery::no_links(&path)?;
    if fs::metadata(&path).map_err(|e| e.to_string())?.len() > 64 * 1024 * 1024 {
        return Err("标注项目过大。".into());
    }
    let project: RecordingProject =
        serde_json::from_slice(&fs::read(path).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
    project.validate()?;
    if video.extension().and_then(|s| s.to_str()) != Some(project.format.as_str()) {
        return Err("项目与原片格式不一致。".into());
    }
    crate::recovery::no_links(&source_path(video))?;
    Ok(Some(project))
}
/// 导出遵守现有裁剪取消和发布机制；新轨道不会覆盖原片、原项目或既有成品。
pub fn export(
    source: &Path,
    output: &Path,
    project: &RecordingProject,
    start_ms: u64,
    end_ms: u64,
    cancel: &AtomicBool,
) -> Result<(), String> {
    project.validate()?;
    if start_ms >= end_ms || end_ms > project.duration_ms {
        return Err("项目导出时间范围无效。".into());
    }
    let layer = project.layer()?;
    if source.extension().and_then(|e| e.to_str()) == Some("gif") {
        let mut options = gif::DecodeOptions::new();
        options.set_color_output(gif::ColorOutput::RGBA);
        let mut decoder = options
            .read_info(fs::File::open(source).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        if (u32::from(decoder.width()), u32::from(decoder.height()))
            != (project.width, project.height)
        {
            return Err("干净 GIF 原片尺寸与项目不一致。".into());
        }
        let mut encoder = super::encoder::create(
            output,
            project.width,
            project.height,
            project.fps,
            0,
            EncodingPreference::Software,
            OutputFormat::Gif,
            false,
        )?;
        let mut clock = 0u64;
        let mut emitted = 0u64;
        while let Some(frame) = decoder.read_next_frame().map_err(|e| e.to_string())? {
            if cancel.load(Ordering::SeqCst) {
                return Err("项目导出已取消。".into());
            }
            // 本程序的干净 GIF 原片始终为完整不透明帧；不将任意增量 GIF 当作完整画面。
            if frame.left != 0
                || frame.top != 0
                || u32::from(frame.width) != project.width
                || u32::from(frame.height) != project.height
                || frame.buffer.len() != project.width as usize * project.height as usize * 4
            {
                return Err("干净 GIF 原片不是完整 RGBA 帧。".into());
            }
            let next = clock + u64::from(frame.delay) * 10;
            while start_ms + emitted * 1000 / u64::from(project.fps) < next.min(end_ms) {
                let ms = start_ms + emitted * 1000 / u64::from(project.fps);
                if ms >= clock {
                    let mut data = frame.buffer.to_vec();
                    for px in data.chunks_exact_mut(4) {
                        px.swap(0, 2);
                        px[3] = 255;
                    }
                    let mut image = frame::BgraImage {
                        width: project.width,
                        height: project.height,
                        data,
                    };
                    layer.apply(&mut image, ms as i64 * 10_000)?;
                    encoder.push(&image, (ms - start_ms) as i64 * 10_000)?;
                }
                emitted += 1;
            }
            clock = next;
            if clock >= end_ms {
                break;
            }
        }
        encoder.finish((end_ms - start_ms) as i64 * 10_000)?;
        return Ok(());
    }
    #[cfg(windows)]
    return windows::export(source, output, project, &layer, start_ms, end_ms, cancel);
    #[cfg(not(windows))]
    {
        let _ = (source, output, layer, cancel);
        Err("当前平台尚不支持 MP4 项目导出。".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn project() -> RecordingProject {
        RecordingProject {
            version: 1,
            width: 16,
            height: 16,
            fps: 30,
            duration_ms: 5000,
            format: "mp4".into(),
            annotations: Vec::new(),
        }
    }
    #[test]
    fn round_trip_keeps_clean_path_local_and_rejects_unknown_fields() {
        let directory = tempfile::tempdir().unwrap();
        let video = directory.path().join("demo.mp4");
        fs::write(&video, b"saved").unwrap();
        fs::write(source_path(&video), b"clean").unwrap();
        save(&video, &project()).unwrap();
        assert_eq!(load(&video).unwrap().unwrap().duration_ms, 5000);
        assert_eq!(
            source_path(&video),
            directory.path().join("demo-source.mp4")
        );
        let mut value = serde_json::to_value(project()).unwrap();
        value["sourcePath"] = serde_json::json!("C:/other.mp4");
        assert!(serde_json::from_value::<RecordingProject>(value).is_err());
    }
    #[test]
    fn invalid_media_time_and_duplicate_ids_are_rejected() {
        let mut value = project();
        value.duration_ms = 0;
        assert!(value.validate().is_err());
        value.duration_ms = 5000;
        let shape=serde_json::from_value(serde_json::json!({"kind":"rectangle","start":{"x":0,"y":0},"end":{"x":10,"y":10},"color":"#ff0000","width":2})).unwrap();
        let track = TimedAnnotation {
            id: "a-1".into(),
            shape,
            start_ms: 1000,
            end_ms: Some(3000),
            fade_out_ms: 500,
            order: 0,
        };
        value.annotations = vec![track.clone(), track];
        assert!(value.validate().is_err());
    }
}
