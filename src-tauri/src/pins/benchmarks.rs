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
            drag_text: None,
            image: image.clone(),
            original: image,
            revision: 0,
            qr_result: None,
            encoded: Default::default(),
            view: PinView::default(),
            annotations: vec![],
            original_annotations: vec![],
        };
        let start = Instant::now();
        let first = encoded_png(&pin.image, &pin.encoded).unwrap();
        let first_ms = start.elapsed().as_secs_f64() * 1000.0;
        let start = Instant::now();
        encoded_png(&pin.image, &pin.encoded).unwrap();
        let cached_ms = start.elapsed().as_secs_f64() * 1000.0;
        frames.push(first);
        pins.push(pin);
        if [1, 4, 8, 12].contains(&count) {
            println!(
                "BENCH {}",
                serde_json::json!({"case":"pins_native","scope":"native_buffers_only_no_webviews","pins":count,
                "rgba_bytes":pins.iter().map(Pin::retained_pixels).sum::<u64>()*4,"encoded_png_bytes":frames.iter().map(|png|png.len()).sum::<usize>(),
                "last_first_read_ms":first_ms,"last_cached_read_ms":cached_ms,"memory":memory()})
            );
        }
    }
    for pin in &pins {
        *pin.encoded.lock().unwrap() = None;
    }
    frames.clear();
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

/// 近似真实截图：浅色底、彩色色块和伪随机“文字行”，避免纯渐变让 PNG 压缩失真。
pub(crate) fn screenshot_like(width: u32, height: u32) -> RgbaImage {
    let mut seed = 0x2545_f491_u32;
    let mut next = move || {
        seed ^= seed << 13;
        seed ^= seed >> 17;
        seed ^= seed << 5;
        seed
    };
    let mut image = RgbaImage::from_pixel(width, height, image::Rgba([246, 247, 249, 255]));
    for y in 0..height {
        let line = y % 28;
        for x in 0..width {
            let block = ((x / 320) + (y / 240)) % 5;
            let pixel = image.get_pixel_mut(x, y);
            if block == 0 {
                *pixel = image::Rgba([40, 110, 200, 255]);
            }
            if (6..18).contains(&line) && (x % 400) < 340 && next() % 3 == 0 {
                let shade = (next() % 90) as u8;
                *pixel = image::Rgba([20 + shade, 24 + shade, 32 + shade, 255]);
            }
        }
    }
    image
}

fn median(mut samples: Vec<f64>) -> f64 {
    samples.sort_by(f64::total_cmp);
    samples[samples.len() / 2]
}

fn timed<T>(f: impl FnOnce() -> T) -> (T, f64) {
    let start = Instant::now();
    let value = f();
    (value, start.elapsed().as_secs_f64() * 1000.0)
}

#[test]
#[ignore = "显式运行贴图底图传输：编码耗时、IPC 序列化耗时与载荷字节"]
fn benchmark_pin_image_transport() {
    for (width, height) in [(1280, 720), (1920, 1080), (3840, 2160)] {
        let mut pin = sample_bench_pin(screenshot_like(width, height));
        let (mut produce, mut serialize, mut bytes) = (vec![], vec![], 0);
        let (mut cached, mut png_bytes) = (vec![], 0);
        for round in 0..5 {
            pin.revision = round;
            pin.encoded = Default::default();
            // 新路径：IPC 只带元信息，首个协议请求编码一次快速 PNG，其后请求复用缓存。
            let ((frame, png), ms) = timed(|| {
                (
                    frame("pin-1", &pin),
                    encoded_png(&pin.image, &pin.encoded).unwrap(),
                )
            });
            produce.push(ms);
            png_bytes = png.len();
            let (_, ms) = timed(|| {
                encoded_png(&pin.image, &pin.encoded)
                    .unwrap()
                    .as_ref()
                    .clone()
            });
            cached.push(ms);
            let (json, ms) = timed(|| serde_json::to_string(&frame).unwrap());
            serialize.push(ms);
            bytes = json.len();
        }
        println!(
            "BENCH {}",
            serde_json::json!({"case":"pin_transport","width":width,"height":height,
            "produce_ms":median(produce),"cached_response_ms":median(cached),"protocol_bytes":png_bytes,
            "ipc_serialize_ms":median(serialize),"ipc_bytes":bytes})
        );
    }
}

fn sample_bench_pin(image: RgbaImage) -> Pin {
    let image = Arc::new(image);
    Pin {
        drag_text: None,
        image: image.clone(),
        original: image,
        revision: 0,
        qr_result: None,
        encoded: Default::default(),
        view: PinView::default(),
        annotations: vec![],
        original_annotations: vec![],
    }
}
