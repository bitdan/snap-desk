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
