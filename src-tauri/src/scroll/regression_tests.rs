//! 内置真实帧经 Recovery 重放；派生场景与外部扩展帧集独立标明来源。
use super::super::recovery::Recovery;
use super::*;
use serde::Deserialize;
use std::path::Path;
use std::time::{Duration, Instant};

// 真值用例只比较位移，置信度和交集由 matcher 的独立测试验证。
fn down(overlap: u32) -> (i32, i32) {
    (0, (568 - overlap) as i32)
}
fn displacement(result: Match) -> (i32, i32) {
    match result {
        Match::Aligned(a) => (a.dx, a.dy),
        other => panic!("预期成功定位，实际 {other:?}"),
    }
}

fn fixture() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/scroll/qrgxj8")
}
fn expected(height: u32) -> RgbaImage {
    image::imageops::crop_imm(&read(&fixture(), "expected.png"), 0, 0, 856, height).to_image()
}

#[test]
fn real_page_replays_through_recovery_without_missing_or_duplicate_rows() {
    let root = fixture();
    let mut stitch = Stitch::new(read(&root, "0000.png")).unwrap();
    let mut recovery = Recovery::default();
    for (index, (overlap, height)) in [
        (101, 1035),
        (335, 1268),
        (334, 1502),
        (102, 1968),
        (334, 2202),
        (335, 2435),
    ]
    .into_iter()
    .enumerate()
    {
        assert_eq!(
            displacement(
                recovery
                    .consider(&mut stitch, read(&root, &format!("{:04}.png", index + 1)))
                    .unwrap()
            ),
            down(overlap)
        );
        assert_eq!(stitch.compose(), expected(height));
    }
    assert_eq!(
        recovery
            .consider(&mut stitch, read(&root, "0006.png"))
            .unwrap(),
        Match::Unchanged
    );
    assert_eq!(stitch.compose(), expected(2435));
}

#[test]
fn real_page_skipped_frame_reconnects_only_before_cache_expiry() {
    let root = fixture();
    for expired in [false, true] {
        let mut stitch = Stitch::new(read(&root, "0000.png")).unwrap();
        let mut recovery = Recovery::default();
        let now = Instant::now();
        assert_eq!(
            recovery
                .consider_at(&mut stitch, read(&root, "0002.png"), now)
                .unwrap(),
            Match::Uncertain
        );
        assert_eq!(stitch.compose(), expected(568));
        let later = now + Duration::from_secs(if expired { 6 } else { 1 });
        assert_eq!(
            displacement(
                recovery
                    .consider_at(&mut stitch, read(&root, "0001.png"), later)
                    .unwrap()
            ),
            down(101)
        );
        assert_eq!(
            stitch.compose(),
            expected(if expired { 1035 } else { 1268 })
        );
    }
}

#[test]
fn real_page_with_derived_fixed_bands_has_no_repeated_header_or_footer() {
    let root = fixture();
    let first = read(&root, "0000.png");
    let with_bands = |mut frame: RgbaImage| {
        for y in (0..24).chain(544..568) {
            for x in 0..856 {
                frame.put_pixel(x, y, *first.get_pixel(x, y));
            }
        }
        frame
    };
    let mut stitch = Stitch::new(with_bands(first.clone())).unwrap();
    let mut recovery = Recovery::default();
    for i in 1..7 {
        assert!(matches!(
            recovery
                .consider(&mut stitch, with_bands(read(&root, &format!("{i:04}.png"))))
                .unwrap(),
            Match::Aligned(_)
        ));
    }
    // 二维版本明确排除固定页头/页脚，只导出实际运动的正文。
    let truth = image::imageops::crop_imm(&expected(2435), 0, 24, 856, 2387).to_image();
    assert_eq!(stitch.compose(), truth);
}

#[test]
fn real_page_derived_loading_placeholder_keeps_anchor_until_content_returns() {
    let root = fixture();
    let mut stitch = Stitch::new(read(&root, "0000.png")).unwrap();
    let mut recovery = Recovery::default();
    let placeholder = RgbaImage::from_pixel(856, 568, image::Rgba([245, 245, 245, 255]));
    assert_eq!(
        recovery.consider(&mut stitch, placeholder).unwrap(),
        Match::Uncertain
    );
    assert_eq!(stitch.compose(), expected(568));
    assert_eq!(
        displacement(
            recovery
                .consider(&mut stitch, read(&root, "0001.png"))
                .unwrap()
        ),
        down(101)
    );
    assert_eq!(stitch.compose(), expected(1035));
}

#[test]
fn real_page_small_animation_does_not_block_stability() {
    let root = fixture();
    let first = read(&root, "0000.png");
    let mut animated = first.clone();
    for y in 120..140 {
        for x in 300..320 {
            animated.put_pixel(x, y, image::Rgba([255, 0, 0, 255]));
        }
    }
    assert!(super::super::stability::stable(&first, &animated));
    let mut stitch = Stitch::new(first).unwrap();
    let mut recovery = Recovery::default();
    let _ = recovery.consider(&mut stitch, animated).unwrap();
    assert_eq!(stitch.compose(), expected(568));
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
