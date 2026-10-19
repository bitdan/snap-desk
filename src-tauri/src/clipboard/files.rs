//! 文件历史只保存绝对路径，不读取或备份源文件；恢复时才检查路径是否仍可用。
pub const MAX_BYTES: usize = 1024 * 1024;
const MAX_FILES: usize = 100;

/// 路径不允许控制字符，因此可用换行分隔持久化，同时支持按完整路径搜索。
pub fn validate(paths: &[String]) -> Result<(), String> {
    if paths.is_empty()
        || paths.len() > MAX_FILES
        || paths.iter().map(|p| p.len() + 1).sum::<usize>() > MAX_BYTES
    {
        return Err("文件历史每条最多 100 个路径，路径总大小不超过 1 MiB。".into());
    }
    for path in paths {
        let bytes = path.as_bytes();
        let drive = bytes.len() >= 3
            && bytes[0].is_ascii_alphabetic()
            && bytes[1] == b':'
            && matches!(bytes[2], b'\\' | b'/');
        let unc = path.starts_with("\\\\")
            && path[2..].split('\\').filter(|s| !s.is_empty()).count() >= 2;
        if (!drive && !unc)
            || path.chars().any(char::is_control)
            || path.starts_with("\\\\.\\")
            || path.encode_utf16().count() > 32767
        {
            return Err("文件引用包含无效路径，未记录。".into());
        }
    }
    Ok(())
}

pub fn from_stored(text: &str) -> Result<Vec<String>, String> {
    if text.len() > MAX_BYTES {
        return Err("文件引用超过大小限制。".into());
    }
    let paths = text.split('\n').map(str::to_owned).collect::<Vec<_>>();
    validate(&paths)?;
    Ok(paths)
}

/// CF_HDROP 的 DROPFILES 头为 20 字节，Unicode 路径列表以两个 UTF-16 NUL 结束。
/// 严格校验偏移和终止符，避免系统剪贴板中的畸形数据引起越界或错误路径。
pub fn decode(bytes: &[u8]) -> Result<Vec<String>, String> {
    if bytes.len() < 24 || bytes.len() > MAX_BYTES {
        return Err("文件剪贴板数据大小无效。".into());
    }
    let offset = u32::from_le_bytes(bytes[..4].try_into().unwrap()) as usize;
    let wide = u32::from_le_bytes(bytes[16..20].try_into().unwrap());
    if wide == 0 {
        return Err("暂不支持旧式 ANSI 文件剪贴板，请在资源管理器重新复制。".into());
    }
    if offset < 20 || offset > bytes.len() - 4 || offset % 2 != 0 {
        return Err("文件剪贴板偏移无效。".into());
    }
    let units = bytes[offset..]
        .chunks_exact(2)
        .map(|b| u16::from_le_bytes([b[0], b[1]]))
        .collect::<Vec<_>>();
    let mut paths = Vec::new();
    let mut start = 0;
    for (index, unit) in units.iter().enumerate() {
        if *unit != 0 {
            continue;
        }
        if index == start {
            validate(&paths)?;
            return Ok(paths);
        }
        paths.push(String::from_utf16(&units[start..index]).map_err(|_| "文件路径编码无效。")?);
        if paths.len() > MAX_FILES {
            return Err("文件历史每条最多 100 个路径。".into());
        }
        start = index + 1;
    }
    Err("文件剪贴板缺少结束标记。".into())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn payload(paths: &[&str]) -> Vec<u8> {
        let mut bytes = vec![0; 20];
        bytes[..4].copy_from_slice(&20u32.to_le_bytes());
        bytes[16..20].copy_from_slice(&1u32.to_le_bytes());
        for path in paths {
            for unit in path.encode_utf16().chain([0]) {
                bytes.extend_from_slice(&unit.to_le_bytes());
            }
        }
        bytes.extend_from_slice(&0u16.to_le_bytes());
        bytes
    }
    #[test]
    fn unicode_multiple_paths_round_trip_without_reading_files() {
        let paths = decode(&payload(&[r"C:\资料\报告😀.pdf", r"D:\图片.png"])).unwrap();
        assert_eq!(paths, from_stored(&paths.join("\n")).unwrap());
        assert_eq!(paths.len(), 2);
    }
    #[test]
    fn rejects_invalid_offsets_terminators_and_limits() {
        let mut data = payload(&[r"C:\file.txt"]);
        data.truncate(data.len() - 2);
        assert!(decode(&data).is_err());
        data[..4].copy_from_slice(&u32::MAX.to_le_bytes());
        assert!(decode(&data).is_err());
        assert!(decode(&payload(&["relative.txt"])).is_err());
        assert!(decode(&payload(&["C:\\bad\nname"])).is_err());
        assert!(decode(&payload(&vec![r"C:\file.txt"; 101])).is_err());
    }
}
