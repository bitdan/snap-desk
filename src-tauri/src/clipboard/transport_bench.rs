//! 历史原图传输基准：素材按入库时的默认 PNG 压缩落盘，只测量读取到 WebView 前的原生开销。
use super::*;
use std::time::Instant;

#[test]
#[ignore = "显式运行历史原图传输：读取、编码耗时与载荷字节"]
fn benchmark_clipboard_image_transport() {
    let dir = std::env::temp_dir().join(format!("snap-desk-clip-bench-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap();
    for (width, height) in [(1920, 1080), (3840, 2160)] {
        let path = dir.join(format!("{width}x{height}.png"));
        crate::pins::benchmarks::screenshot_like(width, height)
            .save_with_format(&path, image::ImageFormat::Png)
            .unwrap();
        let file_bytes = std::fs::metadata(&path).unwrap().len();
        let (mut produce, mut serialize, mut bytes) = (vec![], vec![], 0);
        for _ in 0..5 {
            // 新路径：协议响应体就是磁盘上的 PNG 原字节，不编码、不序列化。
            let start = Instant::now();
            let body = read_limited(
                std::fs::File::open(&path).unwrap(),
                72 * 1024 * 1024,
                "read",
                "size",
            )
            .unwrap();
            produce.push(start.elapsed().as_secs_f64() * 1000.0);
            serialize.push(0.0);
            bytes = body.len();
        }
        produce.sort_by(f64::total_cmp);
        serialize.sort_by(f64::total_cmp);
        println!(
            "BENCH {}",
            serde_json::json!({"case":"clipboard_transport","width":width,"height":height,"file_bytes":file_bytes,
            "produce_ms":produce[2],"ipc_serialize_ms":serialize[2],"ipc_bytes":bytes})
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}
