//! 视频帧始终留在 Rust。CPU 数据独立拥有内存，不借用 WGC 帧池表面。
/// 紧密排列的 BGRA 像素；独立类型防止误传给按 RGBA 解释通道的截图 API。
/// 采集端只去掉 D3D 行尾对齐，编码端直接转换为 NV12。
pub struct BgraImage {
    pub width: u32,
    pub height: u32,
    pub data: Vec<u8>,
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
}

pub struct VideoFrame {
    /// WGC SystemRelativeTime，单位 100 ns，未经暂停时间轴转换。
    pub source_time: i64,
    pub image: BgraImage,
}

/// 队列只携带已投影的时间与代次，编码线程可丢弃暂停前的排队帧。
pub struct QueuedFrame {
    pub generation: u64,
    pub pts: i64,
    pub image: BgraImage,
}
