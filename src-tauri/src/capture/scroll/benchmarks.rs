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

/// 运动帧交给桥接副本；对照基线（未改动前）此处为空操作，与旧采集循环丢弃运动帧一致。
fn feed_transit(recovery: &mut Recovery, stitch: &Stitch, image: RgbaImage) {
    recovery.observe_transit(stitch, image);
}

#[test]
#[ignore = "显式运行快速滚动（静止帧间无重叠、运动帧有重叠）的成功率与耗时"]
fn benchmark_fast_scroll() {
    // 每张静止帧前移 stride 像素；期间每 transit 像素采到一张运动帧。
    for (stride, transit) in [(160u32, 80u32), (300, 100), (480, 160), (720, 180)] {
        let mut stitch = Stitch::with_mode(frame(0), ScrollMode::Vertical).unwrap();
        let mut recovery = Recovery::default();
        let mut accepted = 0;
        let mut rejected = 0;
        let mut transit_ms = Vec::new();
        let mut stable_ms = Vec::new();
        for step in 1..=12u32 {
            let target = step * stride;
            let mut offset = (step - 1) * stride + transit;
            while offset < target {
                let start = Instant::now();
                feed_transit(&mut recovery, &stitch, frame(offset));
                transit_ms.push(ms(start));
                offset += transit;
            }
            let start = Instant::now();
            let result = recovery.consider(&mut stitch, frame(target)).unwrap();
            stable_ms.push(ms(start));
            match result {
                Match::Aligned(_) => accepted += 1,
                _ => rejected += 1,
            }
        }
        // 已提交像素必须逐像素正确，桥接不能引入误拼。
        let output = stitch.compose();
        for (x, y, actual) in output.enumerate_pixels() {
            if actual[3] != 0 {
                assert_eq!(*actual, pixel(x, y + stitch.bounds.y as u32));
            }
        }
        let median = |values: &mut Vec<f64>| {
            values.sort_by(f64::total_cmp);
            values.get(values.len() / 2).copied().unwrap_or(0.0)
        };
        println!(
            "BENCH {}",
            serde_json::json!({"case":"scroll_fast","stride":stride,"transit_step":transit,"accepted":accepted,"rejected":rejected,
            "height":stitch.height,"recovering":recovery.is_recovering(),"transit_median_ms":median(&mut transit_ms),"stable_median_ms":median(&mut stable_ms)})
        );
    }
}

#[test]
#[ignore = "显式运行窗口自带实时读数（类 CPU-Z）时的静止判定与匹配成功率"]
fn benchmark_live_readouts() {
    // 正文为噪声文档，叠加 10 个随文档滚动、每帧刷新的 48×14 读数框。
    let readouts: Vec<(u32, u32)> = (0..10)
        .map(|i| (20 + (i * 97) % 240, 30 + i * 150))
        .collect();
    let source = |tick: u32, offset: u32| {
        RgbaImage::from_fn(320, 240, |x, y| {
            let dy = y + offset;
            if let Some((index, (rx, ry))) = readouts
                .iter()
                .enumerate()
                .find(|(_, (rx, ry))| (*rx..rx + 48).contains(&x) && (*ry..ry + 14).contains(&dy))
            {
                let digit = (x - rx) / 8 + index as u32 * 7;
                let mut v = (digit.wrapping_mul(2654435761)
                    ^ tick.wrapping_mul(40503).wrapping_add(digit))
                .wrapping_mul(2246822519);
                v ^= v >> 13;
                let v = v ^ ((x - rx) % 8 / 2 * 31 + (dy - ry) / 2).wrapping_mul(83492791);
                let shade = if (v >> 7) % 2 == 0 { 25 } else { 230 };
                return Rgba([shade, shade, shade, 255]);
            }
            pixel(x, dy)
        })
    };
    let mut stable_pairs = 0;
    for tick in 0..20 {
        stable_pairs += u32::from(crate::capture::scroll::stability::stable(
            &source(tick, 100),
            &source(tick + 1, 100),
        ));
    }
    let mut stitch = Stitch::with_mode(source(0, 0), ScrollMode::Vertical).unwrap();
    let mut recovery = Recovery::default();
    let mut accepted = 0;
    let mut rejected = 0;
    let mut tick = 1;
    let start = Instant::now();
    // 每个位置先原地刷新一次（用户停顿），再前移 64 像素。
    for step in 0..=20u32 {
        for _ in 0..2 {
            match recovery
                .consider(&mut stitch, source(tick, step * 64))
                .unwrap()
            {
                Match::Uncertain => rejected += 1,
                _ => accepted += 1,
            }
            tick += 1;
        }
    }
    println!(
        "BENCH {}",
        serde_json::json!({"case":"scroll_live_readouts","stable_in_place_pairs":stable_pairs,"in_place_pairs":20,
        "accepted":accepted,"rejected":rejected,"height":stitch.height,"viewport_y":stitch.viewport().y,
        "recovering":recovery.is_recovering(),"duration_ms":ms(start)})
    );
}
