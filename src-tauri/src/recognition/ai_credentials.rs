//! 系统凭证库：Windows 凭据管理器、macOS 钥匙串。密钥按服务地址隔离，改地址不会把旧密钥发送给新服务。
#[cfg(windows)]
mod platform {
    use sha2::{Digest, Sha256};
    use windows::{
        core::{PCWSTR, PWSTR},
        Win32::Security::Credentials::*,
    };

    fn target(endpoint: &str) -> Vec<u16> {
        format!(
            "SnapDesk/translation/{:x}",
            Sha256::digest(endpoint.as_bytes())
        )
        .encode_utf16()
        .chain(Some(0))
        .collect()
    }

    pub fn read(endpoint: &str) -> Result<Option<String>, String> {
        let name = target(endpoint);
        let mut ptr = std::ptr::null_mut();
        // CredRead 分配系统内存；复制后无论解码成功与否都必须 CredFree。
        unsafe {
            if let Err(e) = CredReadW(PCWSTR(name.as_ptr()), CRED_TYPE_GENERIC, None, &mut ptr) {
                if e.code() == windows::core::HRESULT::from_win32(1168) {
                    return Ok(None);
                }
                return Err("无法读取 Windows 凭证库。".into());
            }
            let value = &*ptr;
            let result = if value.CredentialBlobSize == 0 {
                Err("凭证为空，请重新保存。".into())
            } else {
                String::from_utf8(
                    std::slice::from_raw_parts(
                        value.CredentialBlob,
                        value.CredentialBlobSize as usize,
                    )
                    .to_vec(),
                )
                .map(Some)
                .map_err(|_| "凭证格式错误，请重新保存。".into())
            };
            CredFree(ptr.cast());
            result
        }
    }

    pub fn write(endpoint: &str, secret: Option<String>) -> Result<(), String> {
        let mut name = target(endpoint);
        unsafe {
            if let Some(mut secret) = secret {
                if secret.is_empty() || secret.len() > 2048 || secret.chars().any(char::is_control)
                {
                    return Err("密钥为空、过长或含控制字符。".into());
                }
                let credential = CREDENTIALW {
                    Type: CRED_TYPE_GENERIC,
                    TargetName: PWSTR(name.as_mut_ptr()),
                    CredentialBlobSize: secret.len() as u32,
                    CredentialBlob: secret.as_mut_ptr(),
                    Persist: CRED_PERSIST_LOCAL_MACHINE,
                    ..Default::default()
                };
                let result =
                    CredWriteW(&credential, 0).map_err(|_| "保存系统凭证失败。".to_string());
                secret.as_bytes_mut().fill(0);
                result
            } else {
                match CredDeleteW(PCWSTR(name.as_ptr()), CRED_TYPE_GENERIC, None) {
                    Ok(()) => Ok(()),
                    Err(e) if e.code() == windows::core::HRESULT::from_win32(1168) => Ok(()),
                    Err(_) => Err("删除系统凭证失败。".into()),
                }
            }
        }
    }
}

/// macOS 登录钥匙串中的通用密码项：服务名固定，账户名为地址摘要，与 Windows 目标名一一对应。
/// 首次读取时系统可能弹窗询问是否允许访问；用户拒绝会作为读取失败返回，不回退明文。
#[cfg(target_os = "macos")]
mod platform {
    use security_framework::passwords::{
        delete_generic_password, get_generic_password, set_generic_password,
    };
    use sha2::{Digest, Sha256};

    const SERVICE: &str = "io.snapdesk.app";
    /// errSecItemNotFound：条目不存在，视为未保存而不是错误。
    const NOT_FOUND: i32 = -25300;

    fn account(endpoint: &str) -> String {
        format!(
            "SnapDesk/translation/{:x}",
            Sha256::digest(endpoint.as_bytes())
        )
    }

    pub fn read(endpoint: &str) -> Result<Option<String>, String> {
        match get_generic_password(SERVICE, &account(endpoint)) {
            Ok(mut bytes) => {
                let result = if bytes.is_empty() {
                    Err("凭证为空，请重新保存。".into())
                } else {
                    String::from_utf8(bytes.clone())
                        .map(Some)
                        .map_err(|_| "凭证格式错误，请重新保存。".into())
                };
                // 钥匙串返回的副本由本进程持有，用完即清零。
                bytes.fill(0);
                result
            }
            Err(e) if e.code() == NOT_FOUND => Ok(None),
            Err(_) => Err("无法读取 macOS 钥匙串，请确认已允许 Snap Desk 访问。".into()),
        }
    }

    pub fn write(endpoint: &str, secret: Option<String>) -> Result<(), String> {
        let account = account(endpoint);
        if let Some(mut secret) = secret {
            if secret.is_empty() || secret.len() > 2048 || secret.chars().any(char::is_control) {
                return Err("密钥为空、过长或含控制字符。".into());
            }
            // set_generic_password 已存在时原地更新，不会产生重复条目。
            let result = set_generic_password(SERVICE, &account, secret.as_bytes())
                .map_err(|_| "保存到 macOS 钥匙串失败。".to_string());
            // SAFETY: 仅写入 0，字符串仍是合法 UTF-8，随后立即丢弃。
            unsafe { secret.as_bytes_mut().fill(0) };
            result
        } else {
            match delete_generic_password(SERVICE, &account) {
                Ok(()) => Ok(()),
                Err(e) if e.code() == NOT_FOUND => Ok(()),
                Err(_) => Err("删除 macOS 钥匙串条目失败。".into()),
            }
        }
    }
}

// 其余平台尚未接入系统凭证库，明确报错，绝不降级写明文文件。
#[cfg(not(any(windows, target_os = "macos")))]
mod platform {
    pub fn read(_: &str) -> Result<Option<String>, String> {
        Err("此平台尚未接入系统凭证库。".into())
    }
    pub fn write(_: &str, _: Option<String>) -> Result<(), String> {
        Err("此平台尚未接入系统凭证库。".into())
    }
}
pub use platform::{read, write};
