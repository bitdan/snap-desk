//! Windows 系统凭证库。密钥按服务地址隔离，改地址不会把旧密钥发送给新服务。
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

// 首版只交付 Windows；其他平台明确报错，绝不降级写明文文件。
#[cfg(not(windows))]
mod platform {
    pub fn read(_: &str) -> Result<Option<String>, String> {
        Err("此平台尚未接入系统凭证库。".into())
    }
    pub fn write(_: &str, _: Option<String>) -> Result<(), String> {
        Err("此平台尚未接入系统凭证库。".into())
    }
}
pub use platform::{read, write};
