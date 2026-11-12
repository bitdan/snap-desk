//! 真实页面帧集的显式回归入口；默认测试不依赖未提交的本机截图。
use super::*;
use serde::Deserialize;
use std::path::Path;

// 外部帧集的真值只比较位移，像素成图在每个步骤另行核对。
fn displacement(result: Match) -> (i32, i32) {
    match result {
        Match::Aligned(a) => (a.dx, a.dy),
        other => panic!("预期成功定位，实际 {other:?}"),
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    name: String,
    first: String,
    steps: Vec<Step>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Step {
    frame: String,
    outcome: Outcome,
    /// 每一步都提供人工确认的成图，覆盖重复行、漏行、错位与接缝。
    expected: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
enum Outcome {
    Unchanged,
    Uncertain,
    Overlap(u32),
}

fn read(root: &Path, file: &str) -> RgbaImage {
    image::open(root.join(file))
        .unwrap_or_else(|e| panic!("无法读取 {file}: {e}"))
        .to_rgba8()
}

#[test]
#[ignore = "需要 SNAP_DESK_SCROLL_FIXTURES 指向人工标注的真实页面帧集"]
fn real_scroll_sequences_match_reviewed_images() {
    let directory = std::env::var("SNAP_DESK_SCROLL_FIXTURES")
        .expect("请设置 SNAP_DESK_SCROLL_FIXTURES，参见 docs/validation.md");
    let root = Path::new(&directory);
    let cases: Vec<Case> = serde_json::from_slice(
        &std::fs::read(root.join("manifest.json")).expect("缺少 manifest.json"),
    )
    .expect("帧集清单无效");
    assert!(!cases.is_empty(), "真实图片集不能为空");
    for case in cases {
        assert!(!case.steps.is_empty(), "{} 缺少滚动步骤", case.name);
        let mut stitch = Stitch::new(read(root, &case.first)).unwrap();
        for (index, step) in case.steps.into_iter().enumerate() {
            let result = stitch.consider(read(root, &step.frame)).unwrap();
            match step.outcome {
                Outcome::Unchanged => assert_eq!(result, Match::Unchanged),
                Outcome::Uncertain => assert_eq!(result, Match::Uncertain),
                Outcome::Overlap(rows) => assert_eq!(
                    displacement(result),
                    (0, (stitch.frame_height - rows) as i32)
                ),
            }
            let actual = stitch.compose();
            let expected = read(root, &step.expected);
            assert_eq!(
                actual.dimensions(),
                expected.dimensions(),
                "{} 第 {} 步尺寸不符，可能重复或漏行",
                case.name,
                index + 1
            );
            let different = actual
                .pixels()
                .zip(expected.pixels())
                .filter(|(a, b)| a != b)
                .count();
            assert_eq!(
                different,
                0,
                "{} 第 {} 步有 {} 个像素不符",
                case.name,
                index + 1,
                different
            );
        }
    }
}

/// 失去重叠后必须逐段补齐；反向返回可信区时不跳过中间内容。
#[test]
fn fast_scroll_gap_recovers_in_reverse_without_unverified_pixels() {
    use crate::capture::scroll::recovery::Recovery;
    let doc = RgbaImage::from_fn(96, 640, |x, y| {
        let v = (x.wrapping_mul(73856093) ^ y.wrapping_mul(19349663)).wrapping_mul(83492791);
        Rgba([v as u8, (v >> 8) as u8, (v >> 16) as u8, 255])
    });
    let frame = |y| image::imageops::crop_imm(&doc, 0, y, 96, 160).to_image();
    let mut stitch = Stitch::new(frame(0)).unwrap();
    let mut recovery = Recovery::default();
    for offset in [400, 300, 200] {
        assert_eq!(
            recovery.consider(&mut stitch, frame(offset)).unwrap(),
            Match::Uncertain
        );
        assert_eq!(stitch.compose(), frame(0));
    }
    assert!(matches!(
        recovery.consider(&mut stitch, frame(100)).unwrap(),
        Match::Aligned(_)
    ));
    assert!(!recovery.is_recovering());
    assert_eq!(stitch.viewport().y, 100);
    assert_eq!(
        stitch.compose(),
        image::imageops::crop_imm(&doc, 0, 0, 96, 560).to_image()
    );
}

/// 类 CPU-Z 面板：浅灰底、分组框、标签与读数；读数随时间独立变化并随文档滚动。
// (x, y) 为文档坐标，读数块 56×14，分散在多个稳定格与校验分块中。
const READOUTS: [(u32, u32); 8] = [
    (40, 60),
    (220, 90),
    (130, 210),
    (40, 330),
    (230, 380),
    (120, 470),
    (40, 600),
    (220, 650),
];
fn readout(x: u32, y: u32) -> Option<(u32, u32)> {
    READOUTS
        .iter()
        .copied()
        .find(|(rx, ry)| (*rx..rx + 56).contains(&x) && (*ry..ry + 14).contains(&y))
}
fn mix(mut v: u32) -> u32 {
    v ^= v >> 16;
    v = v.wrapping_mul(0x7feb352d);
    v ^= v >> 15;
    v = v.wrapping_mul(0x846ca68b);
    v ^ (v >> 16)
}
fn live_panel(tick: u32) -> RgbaImage {
    RgbaImage::from_fn(320, 900, |x, y| {
        if let Some((rx, ry)) = readout(x, y) {
            // 7 个 8 像素宽的“数字”，每个数字随 tick 独立变化，像真实读数一样只改部分笔画。
            let glyph = (x - rx) / 8;
            let digit = mix(tick * 31 + glyph * 7 + rx * 13 + ry) % 10;
            let v = mix(digit * 1000 + (x - rx) % 8 / 2 * 10 + (y - ry) / 2);
            let shade = if v % 2 == 0 { 20 } else { 235 };
            return Rgba([shade, shade, shade, 255]);
        }
        // 静态标签与边框：每行文字笔画不同，保证文档位置有唯一真值。
        let mut v = (x / 2).wrapping_mul(73856093) ^ (y / 2).wrapping_mul(19349663);
        v = v.wrapping_mul(83492791);
        v ^= v >> 13;
        if x % 80 == 0 || y % 60 == 0 {
            Rgba([90, 90, 110, 255])
        } else if mix(y / 4) % 4 != 0 && v % 3 == 0 {
            Rgba([30, 30, 30, 255])
        } else {
            Rgba([236, 236, 236, 255])
        }
    })
}

fn live_frame(tick: u32, y: u32) -> RgbaImage {
    image::imageops::crop_imm(&live_panel(tick), 0, y, 320, 240).to_image()
}

/// 读数区域以外的像素必须逐像素等于静态真值；读数区允许来自任一时刻。
fn assert_static_truth(stitch: &Stitch, seed_y: i64) {
    let reference = live_panel(0);
    let output = stitch.compose();
    for (x, y, pixel) in output.enumerate_pixels() {
        if pixel[3] == 0 {
            continue;
        }
        let dy = (seed_y + stitch.bounds.y + i64::from(y)) as u32;
        let dx = (stitch.bounds.x + i64::from(x)) as u32;
        if readout(dx, dy).is_none() {
            assert_eq!(
                pixel,
                reference.get_pixel(dx, dy),
                "静态像素 ({x},{y}) 错误"
            );
        }
    }
}

/// 窗口自带实时读数时，原地刷新不能阻止静止判定，也不能让后续滚动匹配失败。
#[test]
fn live_readouts_do_not_block_stability_or_alignment() {
    use crate::capture::scroll::{recovery::Recovery, stability::stable};
    // 原地刷新：同一位置、读数变化，必须视为静止，可继续取样。
    for tick in 0..4 {
        assert!(
            stable(&live_frame(tick, 100), &live_frame(tick + 1, 100)),
            "tick={tick}"
        );
    }
    // 真实滚动仍须判为运动，读数容忍不能放过整页位移。
    for shift in [1, 4, 30, 120] {
        assert!(!stable(&live_frame(0, 100), &live_frame(0, 100 + shift)));
    }
    let mut stitch = Stitch::with_mode(live_frame(0, 0), ScrollMode::Vertical).unwrap();
    let mut recovery = Recovery::default();
    // 原地刷新不应进入恢复状态。
    let result = recovery.consider(&mut stitch, live_frame(1, 0)).unwrap();
    assert!(
        !matches!(result, Match::Uncertain),
        "reason={}",
        stitch.rejection()
    );
    assert!(!recovery.is_recovering());
    let mut tick = 2;
    // 文字行按哈希分布而非固定行距，避免空白行在特定位移下重合被识别为固定栏。
    for y in (50..=650).step_by(50) {
        let result = recovery.consider(&mut stitch, live_frame(tick, y)).unwrap();
        assert!(
            matches!(result, Match::Aligned(a) if a.dy == 50),
            "y={y}, result={result:?}, reason={}",
            stitch.rejection()
        );
        assert_eq!(stitch.viewport().y, i64::from(y));
        tick += 1;
    }
    assert!(!recovery.is_recovering());
    assert!(!stitch.has_holes());
    assert_eq!(stitch.height, 890);
    assert_static_truth(&stitch, 0);
}

fn noise_document() -> RgbaImage {
    RgbaImage::from_fn(320, 1400, |x, y| {
        let mut v = (x.wrapping_mul(73856093) ^ y.wrapping_mul(19349663)).wrapping_mul(83492791);
        v ^= v >> 13;
        Rgba([v as u8, (v >> 8) as u8, (v >> 16) as u8, 255])
    })
}

/// 稍快滚动时，两张静止帧之间已无重叠；等待稳定期间采到的运动帧可以桥接缺口。
#[test]
fn fast_scroll_bridges_gap_with_motion_frames() {
    use crate::capture::scroll::recovery::Recovery;
    let doc = noise_document();
    let frame = |y| image::imageops::crop_imm(&doc, 0, y, 320, 240).to_image();
    // 改动前的失败前提：静止帧相距 720 像素，与可信端点没有任何重叠。
    let mut unbridged = Stitch::with_mode(frame(0), ScrollMode::Vertical).unwrap();
    assert_eq!(
        Recovery::default()
            .consider(&mut unbridged, frame(720))
            .unwrap(),
        Match::Uncertain
    );
    let mut stitch = Stitch::with_mode(frame(0), ScrollMode::Vertical).unwrap();
    let mut recovery = Recovery::default();
    // 首帧未动、一张无法定位的撕裂帧和三张运动帧；无法定位的帧只被跳过。
    let mut torn = frame(200);
    for y in 0..120 {
        for x in 0..320 {
            torn.put_pixel(x, y, *doc.get_pixel(x, 900 + y));
        }
    }
    for transit in [frame(0), frame(180), torn, frame(360), frame(540)] {
        recovery.observe_transit(&stitch, transit);
        // 中间帧只写入桥接副本，可信画布在静止帧确认前保持不变。
        assert_eq!(stitch.compose(), frame(0));
        assert_eq!(stitch.viewport().y, 0);
    }
    assert!(matches!(
        recovery.consider(&mut stitch, frame(720)).unwrap(),
        Match::Aligned(_)
    ));
    assert!(!recovery.is_recovering());
    assert_eq!(stitch.viewport().y, 720);
    assert!(!stitch.has_holes());
    assert_eq!(
        stitch.compose(),
        image::imageops::crop_imm(&doc, 0, 0, 320, 960).to_image()
    );
    // 下一轮从新的可信端点继续，旧桥接副本不会被重复使用。
    assert!(matches!(
        recovery.consider(&mut stitch, frame(800)).unwrap(),
        Match::Aligned(a) if a.dy == 80
    ));
}

/// 运动帧可能尚未重绘完；静止帧在桥接副本上发现冲突时整份丢弃，不写入可信画布。
#[test]
fn unconfirmed_bridge_is_discarded_without_touching_trusted_canvas() {
    use crate::capture::scroll::recovery::Recovery;
    let doc = noise_document();
    let frame = |y| image::imageops::crop_imm(&doc, 0, y, 320, 240).to_image();
    let mut stitch = Stitch::with_mode(frame(0), ScrollMode::Vertical).unwrap();
    let before = stitch.clone();
    let mut recovery = Recovery::default();
    // 新进入的下半部分仍是占位白底（尚未绘制）。
    let mut placeholder = frame(180);
    for y in 140..240 {
        for x in 0..320 {
            placeholder.put_pixel(x, y, Rgba([255, 255, 255, 255]));
        }
    }
    recovery.observe_transit(&stitch, placeholder);
    // 真实静止帧覆盖了占位区：桥接副本上画布冲突，可信端点又无重叠，只能等待恢复。
    assert_eq!(
        recovery.consider(&mut stitch, frame(300)).unwrap(),
        Match::Uncertain
    );
    assert!(recovery.is_recovering());
    assert_eq!(stitch.compose(), before.compose());
    assert_eq!(stitch.viewport(), before.viewport());
    assert_eq!(stitch.count(), before.count());
    // 往回滚到有重叠的位置后照常接续，占位像素从未进入成图。
    assert!(matches!(
        recovery.consider(&mut stitch, frame(150)).unwrap(),
        Match::Aligned(_)
    ));
    assert!(matches!(
        recovery.consider(&mut stitch, frame(300)).unwrap(),
        Match::Aligned(_)
    ));
    assert_eq!(
        stitch.compose(),
        image::imageops::crop_imm(&doc, 0, 0, 320, 540).to_image()
    );
}
