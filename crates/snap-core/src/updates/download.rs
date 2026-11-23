//! 更新包下载：不设总时限，持续 30 秒收不到数据才算卡住；断线或卡住后按 2、4、8、16、30 秒重试，
//! 用 HTTP Range 从已收到的位置续传。设置了下载镜像时先经镜像下载，后面的重试再直连 GitHub。
//! 这里只负责取回字节；调用方必须用 `package::verify` 校验签名后才能保存或安装。
use crate::settings::{update_mirror, update_proxy, UpdateNetwork};
use reqwest::{header, StatusCode, Url};
use std::time::Duration;

/// 正式下载的重试间隔：断线常见于连接 GitHub 文件服务器的网络，稍等再试通常能接上。
pub const RETRY_WAITS: [Duration; 5] = [
    Duration::from_secs(2),
    Duration::from_secs(4),
    Duration::from_secs(8),
    Duration::from_secs(16),
    Duration::from_secs(30),
];
/// 连续这么久没有收到任何数据视为卡住，断开后续传。
pub const STALL_AFTER: Duration = Duration::from_secs(30);
/// 安装包大小上限，防止异常响应占满内存；当前最大的 AppImage 约 100 MiB。
const MAX_BYTES: usize = 512 * 1024 * 1024;

/// 设置中的镜像与代理，已校验。
#[derive(Clone, Debug, Default, PartialEq)]
pub struct Network {
    pub mirror: Option<Url>,
    pub proxy: Option<Url>,
}

impl Network {
    pub fn from_settings(settings: &UpdateNetwork) -> Result<Self, String> {
        Ok(Self {
            mirror: (!settings.mirror.is_empty())
                .then(|| update_mirror(&settings.mirror))
                .transpose()?,
            proxy: (!settings.proxy.is_empty())
                .then(|| update_proxy(&settings.proxy))
                .transpose()?,
        })
    }

    /// 检查与下载共用：设置了代理时只走该代理，否则沿用系统与环境变量中的代理。
    /// `timeout` 只用于版本列表这类小请求；安装包下载不设总时限，由卡住检测兜底。
    pub fn client(&self, timeout: Option<Duration>) -> Result<reqwest::Client, String> {
        let mut builder = reqwest::Client::builder()
            .user_agent("Snap-Desk-Updater")
            .connect_timeout(Duration::from_secs(15));
        if let Some(timeout) = timeout {
            builder = builder.timeout(timeout);
        }
        if let Some(proxy) = &self.proxy {
            builder = builder.proxy(
                reqwest::Proxy::all(proxy.as_str()).map_err(|_| "代理地址无效。".to_string())?,
            );
        }
        builder
            .build()
            .map_err(|_| "无法初始化更新网络客户端。".to_string())
    }

    /// github.com 上的地址加上镜像前缀；没有镜像或不是 GitHub 地址时返回 None。
    pub fn mirrored(&self, url: &Url) -> Option<Url> {
        let mirror = self.mirror.as_ref()?;
        if url.host_str() != Some("github.com") {
            return None;
        }
        Url::parse(&format!("{mirror}{url}")).ok()
    }

    /// 下载地址的尝试顺序：镜像（若有）在前，GitHub 直连在后。
    pub fn routes(&self, url: &Url) -> Vec<Url> {
        self.mirrored(url)
            .into_iter()
            .chain(std::iter::once(url.clone()))
            .collect()
    }
}

/// 第 `attempt` 次尝试使用的地址：把全部尝试平均分给各地址，前面的地址先用完再换下一个。
fn route_index(attempt: usize, attempts: usize, routes: usize) -> usize {
    (attempt * routes / attempts.max(1)).min(routes.saturating_sub(1))
}

#[derive(Debug, PartialEq)]
enum Failure {
    Stalled,
    Connect,
    Timeout,
    Interrupted,
    Status(u16),
    Incomplete,
    TooLarge,
}

impl Failure {
    fn from_reqwest(error: &reqwest::Error) -> Self {
        if error.is_connect() {
            Self::Connect
        } else if error.is_timeout() {
            Self::Timeout
        } else if let Some(status) = error.status() {
            Self::Status(status.as_u16())
        } else {
            Self::Interrupted
        }
    }
    fn code(&self) -> &'static str {
        match self {
            Self::Stalled => "stalled",
            Self::Connect => "connect",
            Self::Timeout => "timeout",
            Self::Interrupted => "interrupted",
            Self::Status(_) => "status",
            Self::Incomplete => "incomplete",
            Self::TooLarge => "too_large",
        }
    }
    fn reason(&self) -> String {
        match self {
            Self::Stalled => "长时间没有收到数据".into(),
            Self::Connect => "无法连接到服务器".into(),
            Self::Timeout => "连接超时".into(),
            Self::Interrupted => "连接中断".into(),
            Self::Status(code) => format!("服务器返回 HTTP {code}"),
            Self::Incomplete => "收到的数据不完整".into(),
            Self::TooLarge => "安装包超过大小上限".into(),
        }
    }
}

/// 最终失败时给出原因、出错的主机和可行的办法；不包含代理账号等设置内容。
fn failure_message(failure: &Failure, url: &Url) -> String {
    format!(
        "下载更新失败：{}（{}）。可稍后重试，在“设置 → 应用更新”中填写代理或下载镜像，或在浏览器中下载安装包。",
        failure.reason(),
        url.host_str().unwrap_or("未知主机")
    )
}

/// 进度回调收到的是累计字节数与总大小（未知时为 None）。
pub async fn download(
    client: &reqwest::Client,
    routes: &[Url],
    waits: &[Duration],
    stall: Duration,
    mut progress: impl FnMut(u64, Option<u64>),
) -> Result<Vec<u8>, String> {
    let first = routes.first().ok_or("没有可用的下载地址。")?;
    let attempts = waits.len() + 1;
    let mut buffer = Vec::new();
    let mut total = None;
    let mut last = (Failure::Interrupted, first.clone());
    for attempt in 0..attempts {
        if attempt > 0 {
            tokio::time::sleep(waits[attempt - 1]).await;
        }
        let url = &routes[route_index(attempt, attempts, routes.len())];
        match fetch(client, url, stall, &mut buffer, &mut total, &mut progress).await {
            Ok(()) => return Ok(buffer),
            Err(failure) => {
                log::warn!(
                    "event=app_update.download_retry module=updater attempt={} received={} reason={}",
                    attempt + 1,
                    buffer.len(),
                    failure.code()
                );
                let fatal = failure == Failure::TooLarge;
                last = (failure, url.clone());
                if fatal {
                    break;
                }
            }
        }
    }
    Err(failure_message(&last.0, &last.1))
}

/// 一次请求：已有数据时用 Range 续传；服务器不支持续传（返回 200）则从头开始。
async fn fetch(
    client: &reqwest::Client,
    url: &Url,
    stall: Duration,
    buffer: &mut Vec<u8>,
    total: &mut Option<u64>,
    progress: &mut impl FnMut(u64, Option<u64>),
) -> Result<(), Failure> {
    let offset = buffer.len() as u64;
    let mut request = client
        .get(url.clone())
        .header(header::ACCEPT, "application/octet-stream");
    if offset > 0 {
        request = request.header(header::RANGE, format!("bytes={offset}-"));
    }
    let mut response = tokio::time::timeout(stall, request.send())
        .await
        .map_err(|_| Failure::Stalled)?
        .map_err(|error| Failure::from_reqwest(&error))?;
    match response.status() {
        StatusCode::PARTIAL_CONTENT if offset > 0 => {
            let (start, size) = content_range(response.headers()).ok_or(Failure::Incomplete)?;
            if start != offset {
                // 服务器从别的位置续传，已收到的数据不能拼接，下一次从头下载。
                buffer.clear();
                return Err(Failure::Incomplete);
            }
            *total = size.or(*total);
        }
        StatusCode::OK => {
            buffer.clear();
            *total = response.content_length();
        }
        StatusCode::RANGE_NOT_SATISFIABLE if offset > 0 && *total == Some(offset) => {
            return Ok(());
        }
        StatusCode::RANGE_NOT_SATISFIABLE => {
            buffer.clear();
            return Err(Failure::Status(416));
        }
        status => return Err(Failure::Status(status.as_u16())),
    }
    if total.is_some_and(|size| size > MAX_BYTES as u64) {
        return Err(Failure::TooLarge);
    }
    progress(buffer.len() as u64, *total);
    loop {
        let chunk = tokio::time::timeout(stall, response.chunk())
            .await
            .map_err(|_| Failure::Stalled)?
            .map_err(|error| Failure::from_reqwest(&error))?;
        let Some(chunk) = chunk else { break };
        if buffer.len() + chunk.len() > MAX_BYTES {
            return Err(Failure::TooLarge);
        }
        buffer.extend_from_slice(&chunk);
        progress(buffer.len() as u64, *total);
    }
    match *total {
        Some(size) if buffer.len() as u64 != size => Err(Failure::Incomplete),
        _ => Ok(()),
    }
}

/// `Content-Range: bytes 100-199/1000` → (起点, 总大小)；总大小为 `*` 时为 None。
fn content_range(headers: &header::HeaderMap) -> Option<(u64, Option<u64>)> {
    let value = headers.get(header::CONTENT_RANGE)?.to_str().ok()?;
    let (range, size) = value.strip_prefix("bytes ")?.split_once('/')?;
    let start = range.split_once('-')?.0.trim().parse().ok()?;
    Some((start, size.trim().parse().ok()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        io::{Read, Write},
        net::TcpListener,
        sync::{Arc, Mutex},
    };

    type Respond = fn(Option<u64>) -> Vec<u8>;
    const BODY: &[u8] = b"0123456789abcdefghij";

    /// 依次处理脚本中的每个连接：读取请求头，记录 Range，按脚本写出响应。
    fn server(script: Vec<Respond>) -> (Url, Arc<Mutex<Vec<Option<u64>>>>) {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let url = Url::parse(&format!(
            "http://{}/app.exe",
            listener.local_addr().unwrap()
        ))
        .unwrap();
        let ranges = Arc::new(Mutex::new(Vec::new()));
        let seen = ranges.clone();
        std::thread::spawn(move || {
            for respond in script {
                let Ok((mut stream, _)) = listener.accept() else {
                    return;
                };
                let mut request = Vec::new();
                let mut byte = [0u8; 1];
                while !request.ends_with(b"\r\n\r\n") && stream.read(&mut byte).unwrap_or(0) == 1 {
                    request.push(byte[0]);
                }
                let text = String::from_utf8_lossy(&request).to_ascii_lowercase();
                let range = text
                    .lines()
                    .find_map(|line| line.strip_prefix("range: bytes="))
                    .and_then(|value| value.trim().trim_end_matches('-').parse().ok());
                seen.lock().unwrap().push(range);
                let _ = stream.write_all(&respond(range));
                let _ = stream.flush();
            }
        });
        (url, ranges)
    }

    fn full(_: Option<u64>) -> Vec<u8> {
        [
            b"HTTP/1.1 200 OK\r\nContent-Length: 20\r\nConnection: close\r\n\r\n".as_slice(),
            BODY,
        ]
        .concat()
    }
    /// 声明 20 字节，只发前 8 字节就断开。
    fn cut_at_8(_: Option<u64>) -> Vec<u8> {
        [
            b"HTTP/1.1 200 OK\r\nContent-Length: 20\r\nConnection: close\r\n\r\n".as_slice(),
            &BODY[..8],
        ]
        .concat()
    }
    fn cut_at_5(_: Option<u64>) -> Vec<u8> {
        [
            b"HTTP/1.1 200 OK\r\nContent-Length: 20\r\nConnection: close\r\n\r\n".as_slice(),
            &BODY[..5],
        ]
        .concat()
    }
    fn partial(range: Option<u64>) -> Vec<u8> {
        let start = range.unwrap() as usize;
        let head = format!(
            "HTTP/1.1 206 Partial Content\r\nContent-Length: {}\r\nContent-Range: bytes {start}-19/20\r\nConnection: close\r\n\r\n",
            20 - start
        );
        [head.as_bytes(), &BODY[start..]].concat()
    }
    fn bad_gateway(_: Option<u64>) -> Vec<u8> {
        b"HTTP/1.1 502 Bad Gateway\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_vec()
    }
    fn unavailable(_: Option<u64>) -> Vec<u8> {
        b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n"
            .to_vec()
    }
    /// 超过卡住时限才发响应头。
    fn silent(_: Option<u64>) -> Vec<u8> {
        std::thread::sleep(Duration::from_millis(700));
        b"HTTP/1.1 200 OK\r\nContent-Length: 20\r\n\r\n".to_vec()
    }

    type Outcome = (Result<Vec<u8>, String>, Vec<(u64, Option<u64>)>);

    fn run(routes: &[Url], attempts: usize) -> Outcome {
        let waits = vec![Duration::ZERO; attempts - 1];
        let client = reqwest::Client::builder().no_proxy().build().unwrap();
        let mut seen = Vec::new();
        let result = crate::updates::block_on(download(
            &client,
            routes,
            &waits,
            Duration::from_millis(500),
            |done, total| seen.push((done, total)),
        ))
        .unwrap();
        (result, seen)
    }

    #[test]
    fn broken_download_resumes_from_where_it_stopped() {
        let (url, ranges) = server(vec![cut_at_8, partial]);
        let (result, progress) = run(&[url], 3);
        assert_eq!(result.unwrap(), BODY);
        assert_eq!(*ranges.lock().unwrap(), vec![None, Some(8)]);
        assert_eq!(progress.last(), Some(&(20, Some(20))));
    }

    #[test]
    fn server_ignoring_range_restarts_from_the_beginning() {
        let (url, ranges) = server(vec![cut_at_5, full]);
        let (result, _) = run(&[url], 2);
        assert_eq!(result.unwrap(), BODY);
        assert_eq!(*ranges.lock().unwrap(), vec![None, Some(5)]);
    }

    #[test]
    fn failing_mirror_falls_back_to_the_direct_route() {
        let (mirror, _) = server(vec![bad_gateway, bad_gateway]);
        let (direct, _) = server(vec![full]);
        let (result, _) = run(&[mirror, direct], 4);
        assert_eq!(result.unwrap(), BODY);
    }

    #[test]
    fn exhausted_retries_explain_the_reason_host_and_remedies() {
        let (url, ranges) = server(vec![unavailable, unavailable, unavailable]);
        let (result, _) = run(&[url], 3);
        let message = result.unwrap_err();
        assert!(message.contains("HTTP 503"), "{message}");
        assert!(message.contains("127.0.0.1"), "{message}");
        assert!(message.contains("代理或下载镜像"), "{message}");
        assert_eq!(ranges.lock().unwrap().len(), 3);
    }

    #[test]
    fn stalled_connection_is_retried() {
        let (url, _) = server(vec![silent, full]);
        let (result, _) = run(&[url], 2);
        assert_eq!(result.unwrap(), BODY);
    }

    #[test]
    fn routes_put_the_mirror_first_and_only_for_github() {
        let url =
            Url::parse("https://github.com/bitdan/snap-desk/releases/download/v1.0.0/app.exe")
                .unwrap();
        let network = Network::from_settings(&UpdateNetwork {
            mirror: "https://mirror.example/gh".into(),
            proxy: String::new(),
        })
        .unwrap();
        let routes = network.routes(&url);
        assert_eq!(
            routes[0].as_str(),
            format!("https://mirror.example/gh/{url}")
        );
        assert_eq!(routes[1], url);
        let other = Url::parse("https://example.com/app.exe").unwrap();
        assert_eq!(network.routes(&other), vec![other]);
        assert_eq!(Network::default().routes(&url), vec![url]);
    }

    #[test]
    fn attempts_are_spread_over_routes_in_order() {
        let order: Vec<_> = (0..6).map(|attempt| route_index(attempt, 6, 2)).collect();
        assert_eq!(order, [0, 0, 0, 1, 1, 1]);
        assert!((0..6).all(|attempt| route_index(attempt, 6, 1) == 0));
    }

    #[test]
    fn content_range_is_parsed_strictly() {
        let mut headers = header::HeaderMap::new();
        headers.insert(header::CONTENT_RANGE, "bytes 8-19/20".parse().unwrap());
        assert_eq!(content_range(&headers), Some((8, Some(20))));
        headers.insert(header::CONTENT_RANGE, "bytes 8-19/*".parse().unwrap());
        assert_eq!(content_range(&headers), Some((8, None)));
        headers.insert(header::CONTENT_RANGE, "items 8-19/20".parse().unwrap());
        assert_eq!(content_range(&headers), None);
    }
}
