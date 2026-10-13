//! 视频帧始终留在 Rust。CPU 数据独立拥有内存，不借用 WGC 帧池表面。
use image::RgbaImage;

pub struct VideoFrame {
    /// WGC SystemRelativeTime，单位 100 ns，未经暂停时间轴转换。
    pub source_time: i64,
    pub image: RgbaImage,
}

/// 队列只携带已投影的时间与代次，编码线程可丢弃暂停前的排队帧。
pub struct QueuedFrame {
    pub generation: u64,
    pub pts: i64,
    pub image: RgbaImage,
}
