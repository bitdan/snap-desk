//! 本机只读图片协议 `snap-image`：贴图底图与历史原图按标签/ID 读取，WebView 直接解码二进制 PNG，
//! 不再经过 Base64 字符串与 JSON 序列化。路径只接受固定格式的编号，不暴露任意文件读取入口。
use image::{
    codecs::png::{CompressionType, FilterType, PngEncoder},
    ImageEncoder, RgbaImage,
};
use std::sync::Arc;
use tauri::{
    http::{Method, Request, Response, StatusCode},
    AppHandle,
};

/// 只在本机协议内传输的 PNG：固定 Sub 滤波和快速压缩，避免逐行试算多种滤波；导出压缩策略不受影响。
pub(crate) fn fast_png(image: &RgbaImage) -> Result<Arc<Vec<u8>>, String> {
    let mut png = Vec::new();
    PngEncoder::new_with_quality(&mut png, CompressionType::Fast, FilterType::Sub)
        .write_image(
            image.as_raw(),
            image.width(),
            image.height(),
            image::ExtendedColorType::Rgba8,
        )
        .map_err(|e| e.to_string())?;
    Ok(Arc::new(png))
}

/// 不缓存屏幕内容；CORS 允许前端画布读取像素，窗口权限由协议入口按请求来源校验。
pub(crate) fn response(status: StatusCode, body: Vec<u8>, length: usize) -> Response<Vec<u8>> {
    Response::builder()
        .status(status)
        .header("Content-Type", "image/png")
        .header("Content-Length", length.to_string())
        .header("Cache-Control", "no-store")
        .header("Access-Control-Allow-Origin", "*")
        .header("X-Content-Type-Options", "nosniff")
        .body(body)
        .expect("fixed image headers")
}

/// 协议载荷：贴图共享常驻的编码缓存，历史原图读取后即归响应所有。
pub(crate) enum Body {
    Shared(Arc<Vec<u8>>),
    Owned(Vec<u8>),
}

#[derive(Debug, PartialEq, Eq)]
pub(crate) enum Route<'a> {
    Pin { label: &'a str, revision: u64 },
    Clip { id: i64 },
}

fn number(text: &str) -> Option<u64> {
    if text.is_empty() || !text.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    text.parse().ok()
}

/// 严格解析 /pin/pin-<n>/<版本>.png 与 /clip/<id>.png；拒绝额外路径、编码路径、负数和溢出。
pub(crate) fn route(path: &str) -> Option<Route<'_>> {
    let mut parts = path.strip_prefix('/')?.split('/');
    let route = match (parts.next()?, parts.next()?, parts.next()) {
        ("pin", label, Some(file)) => {
            number(label.strip_prefix("pin-")?)?;
            Route::Pin {
                label,
                revision: number(file.strip_suffix(".png")?)?,
            }
        }
        ("clip", file, None) => Route::Clip {
            id: i64::try_from(number(file.strip_suffix(".png")?)?).ok()?,
        },
        _ => return None,
    };
    parts.next().is_none().then_some(route)
}

/// 贴图只允许其自身窗口（含它打开的识别子窗口）读取；历史原图只允许历史与主窗口读取。
fn allowed(webview: &str, route: &Route) -> bool {
    let owner = crate::shared::dialog_windows::root_owner(webview);
    match route {
        Route::Pin { label, .. } => owner == *label,
        Route::Clip { .. } => crate::clipboard::IMAGE_WINDOWS.contains(&owner.as_str()),
    }
}

pub fn serve(app: &AppHandle, webview: &str, request: Request<Vec<u8>>) -> Response<Vec<u8>> {
    serve_with(
        |route| allowed(webview, route),
        request,
        |route| match route {
            Route::Pin { label, revision } => {
                crate::pins::protocol_image(app, label, *revision).map(|png| png.map(Body::Shared))
            }
            Route::Clip { id } => {
                crate::clipboard::protocol_image(app, *id).map(|png| png.map(Body::Owned))
            }
        },
    )
}

/// 解析、鉴权与响应独立于 Tauri 窗口，便于用内存素材验证过期版本、越权与 HEAD。
fn serve_with(
    allowed: impl FnOnce(&Route) -> bool,
    request: Request<Vec<u8>>,
    lookup: impl FnOnce(&Route) -> Result<Option<Body>, String>,
) -> Response<Vec<u8>> {
    if request.method() != Method::GET && request.method() != Method::HEAD {
        return response(StatusCode::METHOD_NOT_ALLOWED, vec![], 0);
    }
    let Some(route) = route(request.uri().path()) else {
        return response(StatusCode::NOT_FOUND, vec![], 0);
    };
    if !allowed(&route) {
        return response(StatusCode::FORBIDDEN, vec![], 0);
    }
    let body = match lookup(&route) {
        Ok(Some(body)) => body,
        Ok(None) => return response(StatusCode::NOT_FOUND, vec![], 0),
        Err(error) => {
            log::warn!("event=image_protocol.read_failed module=shared error={error}");
            return response(StatusCode::INTERNAL_SERVER_ERROR, vec![], 0);
        }
    };
    let head = request.method() == Method::HEAD;
    let (body, length) = match body {
        Body::Shared(bytes) => (
            if head { vec![] } else { bytes.as_ref().clone() },
            bytes.len(),
        ),
        Body::Owned(bytes) => {
            let length = bytes.len();
            (if head { vec![] } else { bytes }, length)
        }
    };
    response(StatusCode::OK, body, length)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(method: &str, path: &str) -> Request<Vec<u8>> {
        Request::builder()
            .method(method)
            .uri(path)
            .body(vec![])
            .unwrap()
    }

    #[test]
    fn parses_only_fixed_routes() {
        assert_eq!(
            route("/pin/pin-3/7.png"),
            Some(Route::Pin {
                label: "pin-3",
                revision: 7
            })
        );
        assert_eq!(route("/clip/42.png"), Some(Route::Clip { id: 42 }));
        for path in [
            "/pin/pin-3/7.png/extra",
            "/pin/main/7.png",
            "/pin/pin-/7.png",
            "/pin/pin-3/-1.png",
            "/pin/pin-3/%37.png",
            "/pin/pin-3/7.jpg",
            "/pin/../7.png",
            "/clip/-1.png",
            "/clip/9223372036854775808.png",
            "/clip/1.png/x",
            "/clip/1",
            "/capture/1/2.png",
            "/",
        ] {
            assert_eq!(route(path), None, "{path}");
        }
    }

    #[test]
    fn serves_shared_and_owned_bodies_with_head_and_rejections() {
        let bytes = Arc::new(vec![1, 2, 3]);
        let lookup = |route: &Route| {
            Ok(match route {
                Route::Pin { revision: 2, .. } => Some(Body::Shared(bytes.clone())),
                Route::Clip { id: 5 } => Some(Body::Owned(vec![9; 5])),
                _ => None,
            })
        };
        let get = serve_with(|_| true, request("GET", "/pin/pin-1/2.png"), lookup);
        assert_eq!(get.status(), StatusCode::OK);
        assert_eq!(get.body(), bytes.as_ref());
        assert_eq!(get.headers()["Cache-Control"], "no-store");
        assert_eq!(get.headers()["Access-Control-Allow-Origin"], "*");
        let head = serve_with(|_| true, request("HEAD", "/clip/5.png"), lookup);
        assert!(head.body().is_empty());
        assert_eq!(head.headers()["Content-Length"], "5");
        assert_eq!(
            serve_with(|_| true, request("GET", "/pin/pin-1/1.png"), lookup).status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            serve_with(
                |_| false,
                request("GET", "/pin/pin-1/2.png"),
                |_| panic!("越权窗口不得读取图片")
            )
            .status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            serve_with(|_| true, request("POST", "/clip/5.png"), lookup).status(),
            StatusCode::METHOD_NOT_ALLOWED
        );
        assert_eq!(
            serve_with(
                |_| true,
                request("GET", "/clip/5.png"),
                |_| Err("disk".into())
            )
            .status(),
            StatusCode::INTERNAL_SERVER_ERROR
        );
    }

    #[test]
    fn pin_and_clip_routes_check_their_own_windows() {
        let pin = Route::Pin {
            label: "pin-2",
            revision: 0,
        };
        assert!(allowed("pin-2", &pin));
        assert!(!allowed("pin-1", &pin));
        assert!(!allowed("clipboard", &pin));
        let clip = Route::Clip { id: 1 };
        assert!(allowed("clipboard", &clip));
        assert!(allowed("main", &clip));
        assert!(!allowed("pin-2", &clip));
        assert!(!allowed("shelf", &clip));
    }
}
