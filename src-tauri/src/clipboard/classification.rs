//! 对完整文本做保守分类，不访问网络、磁盘，也不改变原始内容。
use super::model::TextCategory;

/// 只把整段为单个链接、颜色或绝对路径的文本归类，段落中的片段不参与。
pub fn classify(text: &str) -> Option<TextCategory> {
    let text = text.trim();
    if text.is_empty() || text.chars().any(char::is_control) {
        return None;
    }
    if is_link(text) {
        Some(TextCategory::Link)
    } else if is_color(text) {
        Some(TextCategory::Color)
    } else if is_path(text) {
        Some(TextCategory::Path)
    } else {
        None
    }
}

fn is_link(text: &str) -> bool {
    link_url(text).is_some()
}

/// 浏览器打开与分类使用相同校验，拒绝本地文件和可执行协议。
pub(super) fn link_url(text: &str) -> Option<String> {
    let text = text.trim();
    if text.is_empty() || text.chars().any(|c| c.is_whitespace() || c.is_control()) {
        return None;
    }
    let candidate = if text.starts_with("www.") {
        format!("https://{text}")
    } else {
        text.to_owned()
    };
    let url = reqwest::Url::parse(&candidate).ok()?;
    (matches!(url.scheme(), "http" | "https" | "ftp") && url.host_str().is_some())
        .then(|| url.to_string())
}

/// Windows 盘符/UNC 与 Unix 绝对路径；允许资源管理器“复制文件地址”的成对引号。
fn is_path(text: &str) -> bool {
    file_path(text).is_some()
}

/// 返回移除首尾空白和成对引号后的绝对路径文本；不读取或执行文件。
pub(super) fn file_path(text: &str) -> Option<&str> {
    let text = text.trim();
    if text.is_empty() || text.chars().any(char::is_control) {
        return None;
    }
    let path = text
        .strip_prefix('"')
        .and_then(|s| s.strip_suffix('"'))
        .unwrap_or(text);
    let bytes = path.as_bytes();
    let drive = bytes.len() >= 3
        && bytes[0].is_ascii_alphabetic()
        && bytes[1] == b':'
        && matches!(bytes[2], b'\\' | b'/');
    let unc = path.strip_prefix("\\\\").is_some_and(|rest| {
        !rest.starts_with(['.', '?']) && rest.split('\\').filter(|s| !s.is_empty()).count() >= 2
    });
    ((drive || unc || (path.starts_with('/') && !path.starts_with("//")))
        && !path.contains(['<', '>', '|', '"', '*', '?']))
    .then_some(path)
}

/// 支持十六进制与逗号形式 RGB(A)/HSL(A)，数值必须在对应范围内。
fn is_color(text: &str) -> bool {
    if let Some(hex) = text.strip_prefix('#') {
        return matches!(hex.len(), 3 | 4 | 6 | 8) && hex.bytes().all(|b| b.is_ascii_hexdigit());
    }
    let lower = text.to_ascii_lowercase();
    let Some((name, body)) = lower.split_once('(') else {
        return false;
    };
    let Some(body) = body.strip_suffix(')') else {
        return false;
    };
    let count = match name {
        "rgb" | "hsl" => 3,
        "rgba" | "hsla" => 4,
        _ => return false,
    };
    let parts: Vec<_> = body.split(',').map(str::trim).collect();
    if parts.len() != count {
        return false;
    }
    parts.iter().enumerate().all(|(index, part)| {
        let percent = part.ends_with('%');
        let value = part.strip_suffix('%').unwrap_or(part);
        let Ok(value) = value.parse::<f64>() else {
            return false;
        };
        let hsl = name.starts_with("hsl");
        if hsl && index == 0 && percent {
            return false;
        }
        if hsl && matches!(index, 1 | 2) && !percent {
            return false;
        }
        let max = if percent {
            100.0
        } else if index == 3 {
            1.0
        } else if hsl {
            360.0
        } else {
            255.0
        };
        value.is_finite() && (0.0..=max).contains(&value)
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn file_paths_remove_quotes_and_reject_device_or_relative_paths() {
        assert_eq!(
            file_path(r#" "C:\my files\a.txt" "#),
            Some(r"C:\my files\a.txt")
        );
        for text in [
            r"C:relative.txt",
            r"relative\a.txt",
            r"\\.\pipe\name",
            r"\\?\C:\a.txt",
            "C:\\a.txt\nother",
        ] {
            assert!(file_path(text).is_none(), "{text}");
        }
    }

    #[test]
    fn browser_links_normalize_www_and_reject_local_or_executable_protocols() {
        assert_eq!(
            link_url(" www.example.com/a ").as_deref(),
            Some("https://www.example.com/a")
        );
        for text in [
            "javascript:alert(1)",
            "file:///C:/a.txt",
            "data:text/html,test",
            "https://example.com\nignored",
            "https://example.com test",
        ] {
            assert!(link_url(text).is_none(), "{text}");
        }
    }

    #[test]
    fn recognizes_complete_values_without_touching_sources() {
        for text in [
            "https://example.com/a?q=中文",
            "www.example.com",
            "FTP://example.com/a",
        ] {
            assert_eq!(classify(text), Some(TextCategory::Link), "{text}");
        }
        for text in [
            "#abc",
            " #11223344 ",
            "rgb(255, 0, 12)",
            "rgba(10%, 0%, 50%, .5)",
            "hsl(120, 50%, 30%)",
        ] {
            assert_eq!(classify(text), Some(TextCategory::Color), "{text}");
        }
        for text in [
            r"C:\资料\a.txt",
            r#""D:\my files\a.txt""#,
            r"\\server\share\a.txt",
            "/home/user/a.txt",
        ] {
            assert_eq!(classify(text), Some(TextCategory::Path), "{text}");
        }
        for text in [
            "",
            "见 https://example.com",
            "https://",
            "#12",
            "#ggg",
            "rgb(256,0,0)",
            "rgba(1,2,3,2)",
            "hsl(1,2,3)",
            "red",
            "a.txt",
            "C:relative",
            "第一行\n#fff",
        ] {
            assert_eq!(classify(text), None, "{text}");
        }
    }
}
