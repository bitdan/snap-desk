//! 贴图原生缓冲阶梯，不将原生字节数冒充 WebView2 进程占用。
use super::*;
use crate::shared::benchmark::memory;
use std::time::Instant;

#[test]
#[ignore = "显式运行1/4/8/12贴图原生缓冲、版本缓存与关闭释放阶梯"]
fn benchmark_native_pin_ladder() {
    let mut pins = Vec::new();
    let mut frames = Vec::new();
    let mut weak = Vec::new();
    for count in 1..=12 {
        let image = Arc::new(RgbaImage::from_fn(1280, 720, |x, y| {
            image::Rgba([
                ((x / 16) % 200) as u8,
                ((y / 16) % 200) as u8,
                count as u8,
                255,
            ])
        }));
        validate_capacity(
            1280,
            720,
            pins.len(),
            pins.iter().map(Pin::retained_pixels).sum(),
        )
        .unwrap();
        weak.push(Arc::downgrade(&image));
        let pin = Pin {
            image: image.clone(),
            original: image,
            revision: 0,
            qr_result: None,
            view: PinView::default(),
            annotations: vec![],
            original_annotations: vec![],
            group: String::new(),
        };
        let start = Instant::now();
        let first = frame("benchmark", &pin, None).unwrap();
        let first_ms = start.elapsed().as_secs_f64() * 1000.0;
        let start = Instant::now();
        let cached = frame("benchmark", &pin, Some(0)).unwrap();
        let cached_ms = start.elapsed().as_secs_f64() * 1000.0;
        assert!(cached.data_url.is_empty());
        frames.push(first);
        pins.push(pin);
        if [1, 4, 8, 12].contains(&count) {
            println!(
                "BENCH {}",
                serde_json::json!({"case":"pins_native","scope":"native_buffers_only_no_webviews","pins":count,
                "rgba_bytes":pins.iter().map(Pin::retained_pixels).sum::<u64>()*4,"data_url_bytes":frames.iter().map(|frame|frame.data_url.len()).sum::<usize>(),
                "last_first_read_ms":first_ms,"last_cached_read_ms":cached_ms,"memory":memory()})
            );
        }
    }
    for frame in &mut frames {
        drop(std::mem::take(&mut frame.data_url));
    }
    println!(
        "BENCH {}",
        serde_json::json!({"case":"pins_native_hidden","pins":12,"scope":"native_buffers_only_no_webviews","memory":memory()})
    );
    drop(frames);
    drop(pins);
    assert!(weak.iter().all(|image| image.upgrade().is_none()));
    println!(
        "BENCH {}",
        serde_json::json!({"case":"pins_native_closed","pins":0,"memory":memory()})
    );
}
