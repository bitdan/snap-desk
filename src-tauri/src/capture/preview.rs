//! 截图预览的只读协议：仅当前会话及其图片版本有效，不暴露任意文件读取入口。
use super::CaptureState;
use tauri::http::{Method, Request, Response, StatusCode};

/// 严格解析 /会话编号/版本.png；拒绝额外路径、编码路径、负数和溢出。
fn identity(path: &str) -> Option<(u64, u64)> {
    let (id, revision) = path.strip_prefix('/')?.split_once('/')?;
    let revision = revision.strip_suffix(".png")?;
    if id.is_empty()
        || revision.is_empty()
        || !id.bytes().all(|b| b.is_ascii_digit())
        || !revision.bytes().all(|b| b.is_ascii_digit())
    {
        return None;
    }
    Some((id.parse().ok()?, revision.parse().ok()?))
}

/// 不缓存屏幕内容；CORS 允许前端画布读取像素，窗口权限仍由原生协议上下文校验。
fn response(status: StatusCode, body: Vec<u8>, length: usize) -> Response<Vec<u8>> {
    Response::builder()
        .status(status)
        .header("Content-Type", "image/png")
        .header("Content-Length", length.to_string())
        .header("Cache-Control", "no-store")
        .header("Access-Control-Allow-Origin", "*")
        .header("X-Content-Type-Options", "nosniff")
        .body(body)
        .expect("fixed preview headers")
}

/// 先检查调用窗口、方法和会话版本；只在锁内取得 Arc，复制响应体时不阻塞取消/导出。
pub fn serve(state: &CaptureState, allowed: bool, request: Request<Vec<u8>>) -> Response<Vec<u8>> {
    serve_with(allowed, request, |id, revision| {
        state.session.lock().ok().and_then(|session| {
            session
                .as_ref()
                .filter(|s| s.frame.id == id && s.frame.preview_revision == revision)
                .map(|s| s.preview.clone())
        })
    })
}

/// 解析与响应独立于 Tauri 窗口，可用内存素材验证过期版本、HEAD 和缓存行为。
fn serve_with(
    allowed: bool,
    request: Request<Vec<u8>>,
    lookup: impl FnOnce(u64, u64) -> Option<std::sync::Arc<Vec<u8>>>,
) -> Response<Vec<u8>> {
    if !allowed {
        return response(StatusCode::FORBIDDEN, vec![], 0);
    }
    if request.method() != Method::GET && request.method() != Method::HEAD {
        return response(StatusCode::METHOD_NOT_ALLOWED, vec![], 0);
    }
    let Some((id, revision)) = identity(request.uri().path()) else {
        return response(StatusCode::NOT_FOUND, vec![], 0);
    };
    let Some(preview) = lookup(id, revision) else {
        return response(StatusCode::NOT_FOUND, vec![], 0);
    };
    let length = preview.len();
    let body = if request.method() == Method::HEAD {
        vec![]
    } else {
        preview.as_ref().clone()
    };
    response(StatusCode::OK, body, length)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn serves_only_current_version_without_browser_cache_and_supports_head() {
        let bytes = std::sync::Arc::new(vec![1, 2, 3, 4]);
        let lookup = |id, revision| (id == 12 && revision == 3).then(|| bytes.clone());
        let request = |method: &str, path: &str| {
            Request::builder()
                .method(method)
                .uri(path)
                .body(vec![])
                .unwrap()
        };
        let get = serve_with(true, request("GET", "/12/3.png"), lookup);
        assert_eq!(get.status(), StatusCode::OK);
        assert_eq!(get.body(), bytes.as_ref());
        assert_eq!(get.headers()["Cache-Control"], "no-store");
        assert_eq!(get.headers()["Access-Control-Allow-Origin"], "*");
        let head = serve_with(true, request("HEAD", "/12/3.png"), lookup);
        assert!(head.body().is_empty());
        assert_eq!(head.headers()["Content-Length"], "4");
        assert_eq!(
            serve_with(true, request("GET", "/12/2.png"), lookup).status(),
            StatusCode::NOT_FOUND
        );
        assert_eq!(
            serve_with(false, request("GET", "/12/3.png"), |_, _| panic!(
                "禁止窗口不得读取图片"
            ))
            .status(),
            StatusCode::FORBIDDEN
        );
    }

    #[test]
    fn rejects_paths_and_limits_access_to_live_capture() {
        assert_eq!(identity("/12/3.png"), Some((12, 3)));
        for path in [
            "/1/-1.png",
            "/1/2.png/extra",
            "/1/%32.png",
            "/1/.png",
            "/../2.png",
            "/18446744073709551616/0.png",
        ] {
            assert_eq!(identity(path), None);
        }
        let state = CaptureState::default();
        let request = || Request::builder().uri("/12/3.png").body(vec![]).unwrap();
        assert_eq!(
            serve(&state, false, request()).status(),
            StatusCode::FORBIDDEN
        );
        assert_eq!(
            serve(&state, true, request()).status(),
            StatusCode::NOT_FOUND
        );
        let post = Request::builder()
            .method("POST")
            .uri("/12/3.png")
            .body(vec![])
            .unwrap();
        assert_eq!(
            serve(&state, true, post).status(),
            StatusCode::METHOD_NOT_ALLOWED
        );
    }
}
