//! 发布工具：为在线更新包生成 minisign 签名，签名可信注释写入 `version:<版本>`，客户端据此拒绝把旧包伪装成新版本。
//!
//! 密钥与签名沿用既有格式：私钥、公钥、`.sig` 都是 minisign 文本再整体 base64 编码，
//! 与 `crates/snap-core/src/updates/package.rs` 的校验一致。
//!
//! 用法（在 `src-tauri` 下 `cargo run -p snap-release-sign -- <命令>`）：
//! - `sign --app-version <版本> <文件>...`：私钥取自 `SNAP_DESK_SIGNING_PRIVATE_KEY`，
//!   密码取自 `SNAP_DESK_SIGNING_PRIVATE_KEY_PASSWORD`（缺省为空密码），每个文件旁写出 `<文件>.sig`。
//! - `generate <私钥路径>`：生成空密码测试密钥，公钥写到 `<私钥路径>.pub`，只供测试使用。
use base64::{engine::general_purpose::STANDARD, Engine};
use std::fs::File;
use std::io::BufReader;
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{SystemTime, UNIX_EPOCH};

const PRIVATE_KEY: &str = "SNAP_DESK_SIGNING_PRIVATE_KEY";
const PASSWORD: &str = "SNAP_DESK_SIGNING_PRIVATE_KEY_PASSWORD";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let result = match args.first().map(String::as_str) {
        Some("sign") => sign_command(&args[1..]),
        Some("generate") if args.len() == 2 => generate(Path::new(&args[1])),
        _ => Err("用法：snap-release-sign sign --app-version <版本> <文件>... | generate <私钥路径>".into()),
    };
    match result {
        Ok(()) => ExitCode::SUCCESS,
        // 只输出固定提示，不回显密钥或密码。
        Err(message) => {
            eprintln!("{message}");
            ExitCode::FAILURE
        }
    }
}

fn sign_command(args: &[String]) -> Result<(), String> {
    let (version, files) = match args {
        [flag, version, files @ ..] if flag == "--app-version" && !files.is_empty() => (version, files),
        _ => return Err("用法：snap-release-sign sign --app-version <版本> <文件>...".into()),
    };
    let encoded = std::env::var(PRIVATE_KEY).map_err(|_| format!("缺少 {PRIVATE_KEY}。"))?;
    let password = std::env::var(PASSWORD).unwrap_or_default();
    let secret = decode_secret_key(&encoded, password)?;
    for file in files {
        let path = PathBuf::from(file);
        let signature = sign(&secret, &path, version)?;
        std::fs::write(signature_path(&path), signature).map_err(|_| format!("写入签名失败：{file}"))?;
        println!("已签名：{file}");
    }
    Ok(())
}

/// 解出私钥；无论密钥是否加密都显式传入密码，避免 minisign 在 CI 中等待交互输入。
fn decode_secret_key(encoded: &str, password: String) -> Result<minisign::SecretKey, String> {
    const INVALID: &str = "私钥内容或密码不匹配。无密码密钥请删除密码 Secret，并重新复制私钥文件的完整内容。";
    let text = STANDARD.decode(encoded.trim()).map_err(|_| INVALID)?;
    let text = String::from_utf8(text).map_err(|_| INVALID)?;
    minisign::SecretKeyBox::from_string(&text)
        .and_then(|key| key.into_secret_key(Some(password)))
        .map_err(|_| INVALID.into())
}

fn sign(secret: &minisign::SecretKey, path: &Path, version: &str) -> Result<String, String> {
    let name = path
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| format!("文件名无效：{}", path.display()))?;
    let file = File::open(path).map_err(|_| format!("无法读取：{}", path.display()))?;
    let timestamp = SystemTime::now().duration_since(UNIX_EPOCH).map(|d| d.as_secs()).unwrap_or(0);
    let trusted = format!("timestamp:{timestamp}\tfile:{name}\tversion:{version}");
    let signature = minisign::sign(None, secret, BufReader::new(file), Some(&trusted), Some("signature from snap-desk secret key"))
        .map_err(|_| "签名失败。".to_string())?;
    Ok(STANDARD.encode(signature.to_string()))
}

fn signature_path(path: &Path) -> PathBuf {
    let mut name = path.as_os_str().to_owned();
    name.push(".sig");
    PathBuf::from(name)
}

fn generate(path: &Path) -> Result<(), String> {
    let pair = minisign::KeyPair::generate_encrypted_keypair(Some(String::new())).map_err(|_| "生成密钥失败。")?;
    let secret = pair.sk.to_box(None).map_err(|_| "生成密钥失败。")?.to_string();
    let public = pair.pk.to_box().map_err(|_| "生成密钥失败。")?.to_string();
    std::fs::write(path, STANDARD.encode(secret)).map_err(|_| "写入私钥失败。")?;
    std::fs::write(signature_path(path).with_extension("pub"), STANDARD.encode(public)).map_err(|_| "写入公钥失败。")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 与客户端相同的校验：签名有效，且可信注释记录了版本。
    fn verify(public: &str, signature: &str, data: &[u8]) -> String {
        let public = String::from_utf8(STANDARD.decode(public).unwrap()).unwrap();
        let signature = String::from_utf8(STANDARD.decode(signature).unwrap()).unwrap();
        let key = minisign_verify::PublicKey::decode(&public).unwrap();
        let signature = minisign_verify::Signature::decode(&signature).unwrap();
        key.verify(data, &signature, true).unwrap();
        signature.trusted_comment().to_string()
    }

    #[test]
    fn signatures_carry_the_version_and_verify_like_the_client() {
        let dir = std::env::temp_dir().join(format!("snap-release-sign-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let key = dir.join("test.key");
        generate(&key).unwrap();
        let public = std::fs::read_to_string(dir.join("test.key.pub")).unwrap();
        let encoded = std::fs::read_to_string(&key).unwrap();
        let secret = decode_secret_key(&encoded, String::new()).unwrap();
        let payload = dir.join("payload.bin");
        std::fs::write(&payload, b"payload").unwrap();
        let signature = sign(&secret, &payload, "0.3.0-beta.1").unwrap();
        let trusted = verify(&public, &signature, b"payload");
        assert!(trusted.split('\t').any(|field| field == "version:0.3.0-beta.1"));
        assert!(trusted.split('\t').any(|field| field == "file:payload.bin"));
        assert!(decode_secret_key(&encoded, "wrong".into()).is_err());
        assert!(decode_secret_key("invalid", String::new()).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }
}
