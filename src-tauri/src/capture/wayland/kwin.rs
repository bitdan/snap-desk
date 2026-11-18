//! KDE Plasma：KWin 的 `org.kde.KWin.ScreenShot2.CaptureScreen`，不弹出门户选择对话框。
//! 像素经管道写回；读取与 D-Bus 调用并行进行，否则大图会塞满管道而让 KWin 阻塞。
//! KWin 只允许桌面文件声明了 `X-KDE-DBUS-Restricted-Interfaces=org.kde.KWin.ScreenShot2` 的程序调用，
//! 未授权时返回错误，由调用链回退到门户。
use super::{to_rgba, Output, Shot};
use std::collections::HashMap;
use std::io::Read;
use std::os::fd::{AsFd, FromRawFd, OwnedFd};
use std::time::Duration;
use zbus::zvariant::{Fd, OwnedValue, Value};

fn pipe() -> Result<(OwnedFd, OwnedFd), String> {
    let mut fds = [0; 2];
    // SAFETY: pipe2 写入两个新描述符，成功后立即交给 OwnedFd 管理。
    if unsafe { libc::pipe2(fds.as_mut_ptr(), libc::O_CLOEXEC) } != 0 {
        return Err(std::io::Error::last_os_error().to_string());
    }
    Ok(unsafe { (OwnedFd::from_raw_fd(fds[0]), OwnedFd::from_raw_fd(fds[1])) })
}

fn number(metadata: &HashMap<String, OwnedValue>, key: &str) -> Option<u32> {
    metadata
        .get(key)
        .and_then(|value| u32::try_from(value.try_clone().ok()?).ok())
}

/// QImage 格式：4 RGB32、5 ARGB32、6 ARGB32 预乘（小端内存为 BGRA）；16–18 为 RGBX/RGBA8888。
pub(super) fn byte_order(format: Option<u32>) -> Option<bool> {
    match format {
        None | Some(4..=6) => Some(true),
        Some(16..=18) => Some(false),
        Some(_) => None,
    }
}

pub(super) fn capture(layout: &[Output], target: usize, cursor: bool) -> Result<Shot, String> {
    let output = layout.get(target).ok_or("未找到目标显示器。")?.clone();
    let conn = zbus::blocking::connection::Builder::session()
        .map(|builder| builder.method_timeout(Duration::from_secs(10)))
        .and_then(|builder| builder.build())
        .map_err(|e| e.to_string())?;
    let proxy = zbus::blocking::Proxy::new(
        &conn,
        "org.kde.KWin",
        "/org/kde/KWin/ScreenShot2",
        "org.kde.KWin.ScreenShot2",
    )
    .map_err(|e| e.to_string())?;
    let (reader, writer) = pipe()?;
    let limit =
        u64::from(output.physical.0.max(1)) * u64::from(output.physical.1.max(1)) * 4 + (1 << 20);
    let worker = std::thread::Builder::new()
        .name("kwin-screenshot".into())
        .spawn(move || {
            let mut data = Vec::new();
            std::fs::File::from(reader)
                .take(limit.max(64 << 20))
                .read_to_end(&mut data)
                .map(|_| data)
        })
        .map_err(|e| e.to_string())?;
    let mut options: HashMap<&str, Value> = HashMap::new();
    options.insert("native-resolution", Value::from(true));
    options.insert("include-cursor", Value::from(cursor));
    options.insert("hide-caller-windows", Value::from(true));
    let reply: Result<HashMap<String, OwnedValue>, _> = proxy.call(
        "CaptureScreen",
        &(output.name.as_str(), options, Fd::from(writer.as_fd())),
    );
    // 关闭本端写入口；KWin 写完并关闭它持有的副本后读取线程才会结束。
    drop(writer);
    let data = worker
        .join()
        .map_err(|_| "读取 KWin 截图失败。".to_string())?
        .map_err(|e| e.to_string())?;
    let metadata = reply.map_err(|e| e.to_string())?;
    let width = number(&metadata, "width").ok_or("KWin 没有返回宽度。")?;
    let height = number(&metadata, "height").ok_or("KWin 没有返回高度。")?;
    let stride = number(&metadata, "stride").unwrap_or(width.saturating_mul(4));
    let bgra = byte_order(number(&metadata, "format")).ok_or("KWin 返回了不支持的像素格式。")?;
    let image = to_rgba(&data, width, height, stride as usize, bgra, false)?;
    Ok(Shot { output, image })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn qimage_formats_map_to_byte_order() {
        assert_eq!(byte_order(None), Some(true));
        assert_eq!(byte_order(Some(5)), Some(true));
        assert_eq!(byte_order(Some(17)), Some(false));
        assert_eq!(byte_order(Some(13)), None);
    }
}
