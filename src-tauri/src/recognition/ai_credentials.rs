//! 系统凭证库：Windows 凭据管理器、macOS 钥匙串、Linux Secret Service。密钥按服务地址隔离，改地址不会把旧密钥发送给新服务。
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

/// Linux 通过 D-Bus 使用 Secret Service（GNOME Keyring、KWallet、KeePassXC 等）。
/// 条目以固定的 application 属性加地址摘要 account 属性定位，与 Windows 目标名、macOS 账户名一一对应。
/// 会话用 Diffie-Hellman 协商加密，密钥不以明文经过总线；没有密钥环服务时明确报错，绝不降级写文件。
#[cfg(not(any(windows, target_os = "macos")))]
mod platform {
    use dbus_secret_service::{EncryptionType, Error, SecretService};
    use sha2::{Digest, Sha256};
    use std::collections::HashMap;

    const APPLICATION: &str = "io.snapdesk.app";
    /// 密钥环锁定时会弹出系统解锁框；超过两分钟未处理视为取消，避免后台线程一直阻塞。
    const PROMPT_SECONDS: u64 = 120;

    fn account(endpoint: &str) -> String {
        format!(
            "SnapDesk/translation/{:x}",
            Sha256::digest(endpoint.as_bytes())
        )
    }

    fn attributes(account: &str) -> HashMap<&str, &str> {
        HashMap::from([("application", APPLICATION), ("account", account)])
    }

    fn connect() -> Result<SecretService, String> {
        SecretService::connect_with_max_prompt_timeout(EncryptionType::Dh, PROMPT_SECONDS)
            .map_err(|e| describe(&e, "无法连接系统密钥环。"))
    }

    fn describe(error: &Error, fallback: &str) -> String {
        match error {
            Error::Unavailable => {
                "未找到系统密钥环服务，请安装并启动 GNOME Keyring 或 KWallet 等 Secret Service 实现后重试。"
                    .into()
            }
            Error::Prompt => "已取消解锁系统密钥环。".into(),
            Error::Locked => "系统密钥环已锁定，请解锁后重试。".into(),
            _ => fallback.into(),
        }
    }

    pub fn read(endpoint: &str) -> Result<Option<String>, String> {
        let service = connect()?;
        let account = account(endpoint);
        let found = service
            .search_items(attributes(&account))
            .map_err(|e| describe(&e, "无法读取系统密钥环。"))?;
        let item = match (found.unlocked.first(), found.locked.first()) {
            (Some(item), _) => item,
            (None, Some(item)) => {
                item.unlock()
                    .map_err(|e| describe(&e, "无法解锁系统密钥环。"))?;
                item
            }
            (None, None) => return Ok(None),
        };
        let mut bytes = item
            .get_secret()
            .map_err(|e| describe(&e, "无法读取系统密钥环。"))?;
        let result = if bytes.is_empty() {
            Err("凭证为空，请重新保存。".into())
        } else {
            String::from_utf8(bytes.clone())
                .map(Some)
                .map_err(|_| "凭证格式错误，请重新保存。".into())
        };
        bytes.fill(0);
        result
    }

    pub fn write(endpoint: &str, secret: Option<String>) -> Result<(), String> {
        let service = connect()?;
        let account = account(endpoint);
        if let Some(mut secret) = secret {
            if secret.is_empty() || secret.len() > 2048 || secret.chars().any(char::is_control) {
                return Err("密钥为空、过长或含控制字符。".into());
            }
            let result = service
                .get_default_collection()
                .and_then(|collection| {
                    collection.ensure_unlocked()?;
                    // replace = true：同属性的旧条目原地覆盖，不会产生重复项。
                    collection
                        .create_item(
                            "Snap Desk AI 服务密钥",
                            attributes(&account),
                            secret.as_bytes(),
                            true,
                            "text/plain",
                        )
                        .map(|_| ())
                })
                .map_err(|e| match e {
                    // 只写入持久的默认集合；内存中的 session 集合注销后即丢失，不作为退路。
                    Error::NoResult => {
                        "系统密钥环没有默认集合，请在密钥环管理器中创建后重试。".into()
                    }
                    e => describe(&e, "保存到系统密钥环失败。"),
                });
            // SAFETY: 仅写入 0，字符串仍是合法 UTF-8，随后立即丢弃。
            unsafe { secret.as_bytes_mut().fill(0) };
            result
        } else {
            let found = service
                .search_items(attributes(&account))
                .map_err(|e| describe(&e, "删除系统密钥环条目失败。"))?;
            for item in found.unlocked.iter().chain(&found.locked) {
                item.delete()
                    .map_err(|e| describe(&e, "删除系统密钥环条目失败。"))?;
            }
            Ok(())
        }
    }

    #[cfg(test)]
    mod tests {
        use super::*;
        #[test]
        fn entries_are_isolated_per_endpoint() {
            let first = account("openai|https://api.example.com/v1");
            assert_eq!(first, account("openai|https://api.example.com/v1"));
            assert_ne!(first, account("openai|https://other.example.com/v1"));
            assert!(first.starts_with("SnapDesk/translation/"));
            let attributes = attributes(&first);
            assert_eq!(attributes["application"], APPLICATION);
            assert_eq!(attributes["account"], first);
        }
    }
}
pub use platform::{read, write};
