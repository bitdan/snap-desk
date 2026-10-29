//! 有真值的专项基准；耗时不设机器相关阈值，误拼和像素错误必须失败。
use super::*;
use crate::capture::scroll::recovery::Recovery;
use crate::shared::benchmark::memory;
use std::{io::Cursor, time::Instant};

fn pixel(x: u32, y: u32) -> Rgba<u8> {
    let mut value = (x.wrapping_mul(73856093) ^ y.wrapping_mul(19349663)).wrapping_mul(83492791);
    value ^= value >> 13;
    Rgba([value as u8, (value >> 8) as u8, (value >> 16) as u8, 255])
}
fn frame(offset: u32) -> RgbaImage {
    RgbaImage::from_fn(320, 240, |x, y| pixel(x, offset + y))
}
fn ms(start: Instant) -> f64 {
    start.elapsed().as_secs_f64() * 1000.0
}
fn tile_bytes(tile: &Tile) -> usize {
    tile.pixels.capacity() * 4 + tile.covered.capacity() * 8
}

fn recovery_probe(original: &Stitch, offset: u32) -> serde_json::Value {
    let mut stitch = original.clone();
    let mut recovery = Recovery::default();
    let remote = Arc::new(frame(offset + 320));
    let bridge = frame(offset + 160);
    let start = Instant::now();
    assert_eq!(
        recovery.consider(&mut stitch, remote.clone()).unwrap(),
        Match::Uncertain
    );
    let miss_ms = ms(start);
    let start = Instant::now();
    assert_eq!(
        recovery.consider(&mut stitch, remote).unwrap(),
        Match::Uncertain
    );
    let cached_ms = ms(start);
    let start = Instant::now();
    assert!(matches!(
        recovery.consider(&mut stitch, bridge).unwrap(),
        Match::Aligned(_)
    ));
    let recover_ms = ms(start);
    assert_eq!(stitch.viewport().y, i64::from(offset + 160));
    serde_json::json!({"miss_ms":miss_ms,"cached_miss_ms":cached_ms,"recover_ms":recover_ms})
}

#[test]
#[ignore = "显式运行长度、恢复、图块COW、compose、PNG及剪贴板载荷基准"]
fn benchmark_length_ladder() {
    let mut stitch = Stitch::with_mode(frame(0), ScrollMode::Vertical).unwrap();
    let mut recovery = Recovery::default();
    let mut samples = Vec::new();
    for step in 1..=128 {
        // 与真实采集线程一样持有上一份 Arc 快照，观察边界图块的写时复制。
        let snapshot = stitch.clone();
        let input = frame(step * 64);
        let start = Instant::now();
        let result = recovery.consider(&mut stitch, input).unwrap();
        samples.push(ms(start));
        assert!(
            matches!(result, Match::Aligned(_)),
            "step={step}, result={result:?}"
        );
        assert_eq!(stitch.viewport().y, i64::from(step * 64));
        if [4, 16, 64, 128].contains(&step) {
            let cow_bytes: usize = snapshot
                .tiles
                .iter()
                .filter(|(key, tile)| !Arc::ptr_eq(tile, stitch.tiles.get(*key).unwrap()))
                .map(|(_, tile)| tile_bytes(tile))
                .sum();
            let bytes: usize = stitch.tiles.values().map(|tile| tile_bytes(tile)).sum();
            let mut recent = samples[samples.len().saturating_sub(4)..].to_vec();
            recent.sort_by(f64::total_cmp);
            let start = Instant::now();
            let output = stitch.compose();
            let compose_ms = ms(start);
            for (x, y, actual) in output.enumerate_pixels() {
                assert_eq!(*actual, pixel(x, y));
            }
            let start = Instant::now();
            let mut png = Cursor::new(Vec::new());
            output.write_to(&mut png, image::ImageFormat::Png).unwrap();
            let png_ms = ms(start);
            #[cfg(windows)]
            let clipboard = {
                let start = Instant::now();
                let dib = crate::shared::image_io::clipboard_bitmap_for_benchmark(&output).unwrap();
                serde_json::json!({"prepare_ms":ms(start),"dib_bytes":dib.len(),"scope":"payload_only_no_os_write"})
            };
            #[cfg(not(windows))]
            let clipboard = serde_json::Value::Null;
            println!(
                "BENCH {}",
                serde_json::json!({"case":"scroll_length","steps":step,"height":stitch.height,"match_recent_median_ms":recent[recent.len()/2],
                "tiles":stitch.tiles.len(),"anchors":stitch.anchors.len(),"tile_bytes":bytes,"cow_bytes":cow_bytes,"snapshot_tile_bytes":bytes+cow_bytes,
                "compose_ms":compose_ms,"rgba_bytes":output.as_raw().len(),"png_ms":png_ms,"png_bytes":png.get_ref().len(),"clipboard":clipboard,"recovery":recovery_probe(&stitch,step*64),"memory":memory()})
            );
        }
    }
}

#[test]
#[ignore = "显式运行固定表头、周期纹理和局部动态内容的拒绝/误拼统计"]
fn benchmark_rejection_matrix() {
    for scenario in ["normal", "fixed_header", "repeated", "dynamic"] {
        let source = |offset: u32| {
            RgbaImage::from_fn(320, 240, |x, y| {
                if scenario == "repeated" {
                    return Rgba([
                        ((x % 16) * 11) as u8,
                        (((y + offset) % 16) * 13) as u8,
                        80,
                        255,
                    ]);
                }
                if scenario == "fixed_header" && y < 24 {
                    return pixel(x, y);
                }
                if scenario == "dynamic" && (80..144).contains(&x) && (80..144).contains(&y) {
                    return pixel(x.wrapping_add(offset * 3), y);
                }
                pixel(x, y + offset)
            })
        };
        let mut stitch = Stitch::with_mode(source(0), ScrollMode::Vertical).unwrap();
        let mut accepted = 0;
        let mut rejected = 0;
        let mut false_matches = 0;
        let start = Instant::now();
        for step in 1..=12 {
            match stitch.consider(source(step * 40)).unwrap() {
                Match::Aligned(_) => {
                    accepted += 1;
                    let expected =
                        i64::from(step * 40) + if scenario == "fixed_header" { 24 } else { 0 };
                    if stitch.viewport().y != expected || scenario == "repeated" {
                        false_matches += 1;
                    }
                }
                _ => rejected += 1,
            }
        }
        println!(
            "BENCH {}",
            serde_json::json!({"case":"scroll_rejection","scenario":scenario,"accepted":accepted,"rejected":rejected,"false_matches":false_matches,"duration_ms":ms(start),"memory":memory()})
        );
        assert_eq!(false_matches, 0, "{scenario}");
        if scenario == "normal" || scenario == "fixed_header" {
            assert_eq!(accepted, 12, "{scenario}");
        }
    }
}
