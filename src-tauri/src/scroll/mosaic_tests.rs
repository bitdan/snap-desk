//! 合成真值按二维坐标裁帧，逐像素核对覆盖区；绝不把空洞当成已观察内容。
use super::*;

fn document() -> RgbaImage {
    RgbaImage::from_fn(700, 700, |x, y| {
        let mut v = (x.wrapping_mul(73856093) ^ y.wrapping_mul(19349663)).wrapping_mul(83492791);
        v ^= v >> 13;
        Rgba([v as u8, (v >> 8) as u8, (v >> 16) as u8, 255])
    })
}
fn frame(doc: &RgbaImage, x: u32, y: u32) -> RgbaImage {
    image::imageops::crop_imm(doc, x, y, 160, 160).to_image()
}
fn moved(result: Match, dx: i32, dy: i32) {
    let Match::Aligned(a) = result else {
        panic!("预期 ({dx},{dy})，实际 {result:?}")
    };
    assert_eq!((a.dx, a.dy), (dx, dy));
    assert!(a.confidence >= 0.9);
    assert!(a.overlap.width >= 32 && a.overlap.height >= 32);
}
fn assert_truth(stitch: &Stitch, doc: &RgbaImage, seed: (i64, i64)) {
    let output = stitch.compose();
    for (x, y, pixel) in output.enumerate_pixels() {
        if pixel[3] != 0 {
            assert_eq!(
                pixel,
                doc.get_pixel(
                    (seed.0 + stitch.bounds.x + i64::from(x)) as u32,
                    (seed.1 + stitch.bounds.y + i64::from(y)) as u32
                ),
                "像素 ({x},{y}) 不匹配"
            );
        }
    }
}

#[test]
fn all_axes_reverse_and_diagonal_preserve_document_pixels() {
    let doc = document();
    let mut stitch = Stitch::new(frame(&doc, 200, 200)).unwrap();
    let mut previous = (200, 200);
    for (x, y) in [
        (200, 300),
        (300, 300),
        (300, 220),
        (210, 250),
        (110, 250),
        (110, 150),
        (200, 200),
    ] {
        moved(
            stitch.consider(frame(&doc, x, y)).unwrap(),
            x as i32 - previous.0,
            y as i32 - previous.1,
        );
        previous = (x as i32, y as i32);
        assert_truth(&stitch, &doc, (200, 200));
    }
    assert!(stitch.bounds.x < 0 && stitch.bounds.y < 0);
    assert_eq!(stitch.viewport().x, 0);
    assert_eq!(stitch.viewport().y, 0);
}
#[test]
fn l_path_leaves_transparent_hole_and_snake_fills_it() {
    let doc = document();
    let mut stitch = Stitch::new(frame(&doc, 0, 0)).unwrap();
    moved(stitch.consider(frame(&doc, 0, 100)).unwrap(), 0, 100);
    moved(stitch.consider(frame(&doc, 100, 100)).unwrap(), 100, 0);
    assert!(stitch.has_holes());
    assert_eq!(stitch.compose().get_pixel(230, 30)[3], 0);
    assert!(
        (stitch.coverage() - 100.0 * (260.0 * 260.0 - 10000.0) / (260.0 * 260.0)).abs() < 0.001
    );
    moved(stitch.consider(frame(&doc, 100, 0)).unwrap(), 0, -100);
    assert!(!stitch.has_holes());
    assert_eq!(
        stitch.compose(),
        image::imageops::crop_imm(&doc, 0, 0, 260, 260).to_image()
    );
    let before = stitch.compose();
    let count = stitch.count();
    moved(stitch.consider(frame(&doc, 0, 0)).unwrap(), -100, 0);
    assert_eq!(stitch.compose(), before);
    assert_eq!(stitch.count(), count);
    assert_eq!(
        stitch.consider(frame(&doc, 0, 0)).unwrap(),
        Match::Unchanged
    );
}
#[test]
fn historical_anchor_relocates_without_overlap_with_latest() {
    let doc = document();
    let mut stitch = Stitch::new(frame(&doc, 0, 0)).unwrap();
    for x in [100, 200, 300] {
        assert!(matches!(
            stitch.consider(frame(&doc, x, 0)).unwrap(),
            Match::Aligned(_)
        ));
    }
    let before = stitch.compose();
    moved(stitch.consider(frame(&doc, 0, 0)).unwrap(), -300, 0);
    assert_eq!(stitch.compose(), before);
    assert_eq!(stitch.viewport().x, 0);
}

#[test]
fn spatial_anchor_budget_preserves_the_start_of_a_long_path() {
    let doc = document();
    let mut stitch = Stitch::new(frame(&doc, 0, 0)).unwrap();
    for x in (40..=520).step_by(40) {
        moved(stitch.consider(frame(&doc, x, 0)).unwrap(), 40, 0);
        assert!(stitch.anchors.len() <= 6);
    }
    let before = stitch.compose();
    moved(stitch.consider(frame(&doc, 0, 0)).unwrap(), -520, 0);
    assert_eq!(stitch.compose(), before);
}
#[test]
fn fixed_header_footer_and_frozen_column_are_excluded_from_content() {
    let doc = document();
    let fixed = |x, y| {
        let mut image = frame(&doc, x, y);
        for py in 0..160 {
            for px in 0..160 {
                if py < 12 || py >= 148 || px < 16 {
                    image.put_pixel(px, py, *doc.get_pixel(px + 400, py + 400));
                }
            }
        }
        image
    };
    let mut stitch = Stitch::new(fixed(100, 100)).unwrap();
    moved(stitch.consider(fixed(100, 180)).unwrap(), 0, 80);
    moved(stitch.consider(fixed(180, 180)).unwrap(), 80, 0);
    assert_eq!(
        stitch.content,
        Content {
            left: 16,
            top: 12,
            right: 160,
            bottom: 148
        }
    );
    assert_truth(&stitch, &doc, (100, 100));
    assert!(stitch.has_holes());
}
#[test]
fn established_fixed_bands_survive_axis_changes_and_reverse_paths() {
    let doc = document();
    // 表头随横向滚动、冻结列随纵向滚动，因此它们并非始终在屏幕坐标静止。
    // 先沿垂直于固定栏的方向识别边界，再切换方向与折返，边界不能重新扩张。
    for columns in [false, true] {
        let fixed = |x, y| {
            let mut image = frame(&doc, x, y);
            for py in 0..160 {
                for px in 0..160 {
                    if columns && (px < 12 || px >= 148) {
                        image.put_pixel(px, py, *doc.get_pixel(400 + px, y + py));
                    } else if !columns && (py < 12 || py >= 148) {
                        image.put_pixel(px, py, *doc.get_pixel(x + px, 400 + py));
                    }
                }
            }
            image
        };
        let path = if columns {
            [(180, 100), (180, 180), (140, 180), (100, 100)]
        } else {
            [(100, 180), (180, 180), (180, 140), (100, 100)]
        };
        let mut stitch = Stitch::new(fixed(100, 100)).unwrap();
        let mut previous = (100, 100);
        for (x, y) in path {
            moved(
                stitch.consider(fixed(x, y)).unwrap(),
                x as i32 - previous.0,
                y as i32 - previous.1,
            );
            previous = (x as i32, y as i32);
            assert_eq!(
                stitch.content,
                if columns {
                    Content {
                        left: 12,
                        top: 0,
                        right: 148,
                        bottom: 160,
                    }
                } else {
                    Content {
                        left: 0,
                        top: 12,
                        right: 160,
                        bottom: 148,
                    }
                }
            );
            assert_truth(&stitch, &doc, (100, 100));
        }
        // 当前帧无重叠时也通过起点历史锚点重定位；不能退回全选区匹配。
        for (x, y) in [(180, 100), (260, 100), (340, 100)] {
            moved(stitch.consider(fixed(x, y)).unwrap(), 80, 0);
        }
        let before = stitch.compose();
        moved(stitch.consider(fixed(100, 100)).unwrap(), -240, 0);
        assert_eq!(stitch.compose(), before);
        assert_truth(&stitch, &doc, (100, 100));
    }
}

#[test]
fn newly_appearing_fixed_band_does_not_rewrite_confirmed_canvas() {
    let doc = document();
    let mut stitch = Stitch::new(frame(&doc, 100, 100)).unwrap();
    moved(stitch.consider(frame(&doc, 100, 180)).unwrap(), 0, 80);
    let before = stitch.compose();
    let viewport = stitch.viewport();
    let revision = stitch.match_revision();
    let mut next = frame(&doc, 100, 240);
    for y in 0..12 {
        for x in 0..160 {
            next.put_pixel(x, y, *stitch.last().get_pixel(x, y));
        }
    }
    assert_eq!(stitch.consider(next).unwrap(), Match::Uncertain);
    assert_eq!(stitch.compose(), before);
    assert_eq!(stitch.viewport(), viewport);
    assert_eq!(stitch.match_revision(), revision);
}

#[test]
fn unsupported_motion_patterns_and_size_changes_do_not_modify_canvas() {
    let doc = document();
    for mode in [ScrollMode::Vertical, ScrollMode::Horizontal] {
        let mut stitch = Stitch::with_mode(frame(&doc, 100, 100), mode).unwrap();
        let before = stitch.compose();
        assert_eq!(
            stitch.consider(frame(&doc, 160, 160)).unwrap(),
            Match::Uncertain
        );
        assert_eq!(stitch.compose(), before);
    }
    let mut stitch = Stitch::new(frame(&doc, 0, 0)).unwrap();
    let before = stitch.compose();
    assert_eq!(
        stitch.consider(frame(&doc, 400, 400)).unwrap(),
        Match::Uncertain
    );
    assert!(stitch.consider(RgbaImage::new(161, 160)).is_err());
    assert_eq!(stitch.compose(), before);
    let blank = RgbaImage::from_pixel(160, 160, Rgba([255, 255, 255, 255]));
    assert_eq!(stitch.consider(blank).unwrap(), Match::Uncertain);
    // 两个面板不同步移动，不能选一半拼接整张选区。
    let mut split = frame(&doc, 0, 60);
    let other = frame(&doc, 0, 90);
    for y in 0..160 {
        for x in 80..160 {
            split.put_pixel(x, y, *other.get_pixel(x, y));
        }
    }
    assert_eq!(stitch.consider(split).unwrap(), Match::Uncertain);
    assert_eq!(stitch.compose(), before);
}
#[test]
fn repeated_grid_is_not_position_evidence() {
    let repeat = RgbaImage::from_fn(160, 320, |x, y| {
        Rgba([((x % 20) * 12) as u8, ((y % 20) * 12) as u8, 0, 255])
    });
    let a = image::imageops::crop_imm(&repeat, 0, 0, 160, 160).to_image();
    let b = image::imageops::crop_imm(&repeat, 0, 5, 160, 160).to_image();
    let mut stitch = Stitch::new(a.clone()).unwrap();
    assert_eq!(stitch.consider(b).unwrap(), Match::Uncertain);
    assert_eq!(stitch.compose(), a);
}
#[test]
fn capacity_rejection_and_snapshot_copy_on_write_preserve_pixels() {
    let doc = document();
    let mut stitch = Stitch::new(frame(&doc, 0, 0)).unwrap();
    let snapshot = stitch.clone();
    let far = Anchor {
        image: Arc::new(frame(&doc, 0, 0)),
        x: 60000,
        y: 0,
    };
    assert!(stitch.write(far, stitch.content).is_err());
    assert_eq!(stitch.compose(), snapshot.compose());
    moved(stitch.consider(frame(&doc, 100, 0)).unwrap(), 100, 0);
    assert_eq!(snapshot.compose(), frame(&doc, 0, 0));
    assert_eq!(
        stitch.preview(stitch.width, stitch.height),
        stitch.compose()
    );
    assert_eq!(stitch.preview(130, 80).dimensions(), (130, 80));
    assert!(Stitch::new(RgbaImage::new(31, 64)).is_err());
}
#[test]
fn one_pixel_and_minimum_overlap_are_exact() {
    let doc = document();
    for (dx, dy) in [
        (1, 0),
        (0, 1),
        (128, 0),
        (0, 128),
        (-128, 0),
        (0, -128),
        (70, -50),
    ] {
        let mut stitch = Stitch::new(frame(&doc, 200, 200)).unwrap();
        moved(
            stitch
                .consider(frame(&doc, (200 + dx) as u32, (200 + dy) as u32))
                .unwrap(),
            dx,
            dy,
        );
        assert_truth(&stitch, &doc, (200, 200));
    }
    let mut stitch = Stitch::new(frame(&doc, 0, 0)).unwrap();
    assert_eq!(
        stitch.consider(frame(&doc, 129, 0)).unwrap(),
        Match::Uncertain
    );
}

#[test]
fn tiled_write_preserves_full_tiles_and_negative_coordinate_holes() {
    let doc = document();
    let image = image::imageops::crop_imm(&doc, 100, 100, 320, 320).to_image();
    let mut stitch = Stitch::new(image.clone()).unwrap();
    let original = stitch.clone();
    let next = Anchor {
        image: Arc::new(image::imageops::crop_imm(&doc, 20, 40, 320, 320).to_image()),
        x: -80,
        y: -60,
    };
    stitch.write(next.clone(), stitch.content).unwrap();
    assert_eq!(stitch.covered, 320 * 320 * 2 - 240 * 260);
    assert_truth(&stitch, &doc, (100, 100));
    assert_eq!(stitch.pixel(-70, 310)[3], 0);
    assert!(Arc::ptr_eq(
        &stitch.tiles[&(0, 0)],
        &original.tiles[&(0, 0)]
    ));
    assert_eq!(original.compose(), image);
    let before = stitch.clone();
    stitch.write(next, stitch.content).unwrap();
    assert_eq!(stitch.covered, before.covered);
    for (key, tile) in &stitch.tiles {
        assert!(Arc::ptr_eq(tile, &before.tiles[key]));
        assert_eq!(
            tile.filled,
            tile.covered
                .iter()
                .map(|bits| bits.count_ones() as usize)
                .sum::<usize>()
        );
    }
}
