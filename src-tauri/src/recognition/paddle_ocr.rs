//! 同一 Paddle 服务兼容官方异步任务和旧同步结果；截图只上传图片，不下载结果中的可视化图。
use super::services::RecognitionService;
use serde_json::{json, Value};
use std::time::{Duration, Instant};

const JOB_PATH: &str = "/api/v2/ocr/jobs";
const MAX_TEXT: usize = 1024 * 1024;

pub(super) fn is_jobs_endpoint(endpoint: &str) -> bool {
    reqwest::Url::parse(endpoint).is_ok_and(|url| url.path().trim_end_matches('/') == JOB_PATH)
}

pub(super) fn supports_model(endpoint: &str, model: &str) -> bool {
    match model {
        "PP-OCRv5" | "PP-OCRv6" => true,
        "PaddleOCR-VL-1.6" | "PP-StructureV3" => is_jobs_endpoint(endpoint),
        _ => false,
    }
}

/// 整个任务最多两分钟，单次网络请求也受剩余时间约束，避免服务一直 pending 导致无限等待。
fn remaining(deadline: Instant) -> Result<Duration, String> {
    deadline
        .checked_duration_since(Instant::now())
        .filter(|value| !value.is_zero())
        .map(|value| value.min(Duration::from_secs(60)))
        .ok_or_else(|| "PaddleOCR 任务超时，请稍后重试。".into())
}

/// 使用固定字段和文件名构造 multipart，无需把截图转成外部可访问 URL。
/// 模型已通过白名单校验；边界检查确保图片内容不会意外截断上传。
fn multipart(image: &[u8], model: &str) -> (String, Vec<u8>) {
    let digest = blake3::hash(image).to_hex();
    let boundary = loop {
        let candidate = format!(
            "SnapDesk-{}-{}",
            &digest.as_str()[..24],
            super::ocr::next_request()
        );
        if !image
            .windows(candidate.len())
            .any(|part| part == candidate.as_bytes())
        {
            break candidate;
        }
    };
    let mut options = json!({"useDocOrientationClassify": false, "useDocUnwarping": false});
    if model.starts_with("PP-") {
        options["useTextlineOrientation"] = false.into();
    }
    let mut body = Vec::new();
    for (name, value) in [
        ("model", model.to_string()),
        ("optionalPayload", options.to_string()),
    ] {
        body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"{name}\"\r\n\r\n{value}\r\n").as_bytes());
    }
    body.extend_from_slice(format!("--{boundary}\r\nContent-Disposition: form-data; name=\"file\"; filename=\"screenshot.png\"\r\nContent-Type: image/png\r\n\r\n").as_bytes());
    body.extend_from_slice(image);
    body.extend_from_slice(format!("\r\n--{boundary}--\r\n").as_bytes());
    (format!("multipart/form-data; boundary={boundary}"), body)
}

fn job_data(bytes: &[u8]) -> Result<Value, String> {
    let value: Value = serde_json::from_slice(bytes).map_err(|_| "PaddleOCR 任务响应格式无效。")?;
    if value.get("code").and_then(Value::as_i64) != Some(0) {
        return Err("PaddleOCR 任务请求失败，请检查 Token、模型权限和额度。".into());
    }
    value
        .get("data")
        .filter(|value| value.is_object())
        .cloned()
        .ok_or_else(|| "PaddleOCR 任务响应缺少数据。".into())
}

fn job_url(endpoint: &str, id: &str) -> Result<String, String> {
    if id.is_empty()
        || id.len() > 200
        || !id
            .bytes()
            .all(|c| c.is_ascii_alphanumeric() || c == b'-' || c == b'_')
    {
        return Err("PaddleOCR 返回的任务编号无效。".into());
    }
    Ok(format!("{}/{id}", endpoint.trim_end_matches('/')))
}

/// 结果地址可能带 BOS 签名参数，因此不复用禁止查询参数的服务配置校验。
/// 下载使用无认证请求且禁止重定向，AI Studio Token 只发送给用户配置的任务接口。
fn result_url(value: &str) -> Result<reqwest::Url, String> {
    let url = reqwest::Url::parse(value).map_err(|_| "PaddleOCR 结果下载地址无效。")?;
    let host = url.host_str().ok_or("PaddleOCR 结果下载地址无效。")?;
    if value.len() > 16_384
        || url.scheme() != "https"
        || !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || matches!(host, "localhost" | "127.0.0.1" | "[::1]")
    {
        return Err("PaddleOCR 结果下载地址无效。".into());
    }
    Ok(url)
}

/// 提交一次，随后仅查询同一任务；请求失败不会自动重复上传和消耗额度。
pub(super) fn recognize(
    client: &reqwest::blocking::Client,
    service: &RecognitionService,
    token: &str,
    image: &[u8],
) -> Result<String, String> {
    let deadline = Instant::now() + Duration::from_secs(120);
    let (content_type, body) = multipart(image, &service.model);
    let response = client
        .post(super::translation::endpoint_url(&service.endpoint)?)
        .bearer_auth(token)
        .header(reqwest::header::CONTENT_TYPE, content_type)
        .timeout(remaining(deadline)?)
        .body(body)
        .send()
        .map_err(|_| "提交 PaddleOCR 任务失败或超时。")?;
    let submitted = job_data(&super::online_ocr::read_response(response)?)?;
    let id = submitted
        .get("jobId")
        .and_then(Value::as_str)
        .ok_or("PaddleOCR 响应缺少任务编号。")?;
    let poll_url = job_url(&service.endpoint, id)?;
    loop {
        let response = client
            .get(&poll_url)
            .bearer_auth(token)
            .timeout(remaining(deadline)?)
            .send()
            .map_err(|_| "查询 PaddleOCR 任务失败或超时。")?;
        let data = job_data(&super::online_ocr::read_response(response)?)?;
        // 兼容官方文档中直接返回 data.state 和包在 extractResult 中的响应。
        let data = data
            .get("extractResult")
            .filter(|value| value.get("state").is_some())
            .unwrap_or(&data);
        match data.get("state").and_then(Value::as_str) {
            Some("pending" | "running") => {
                std::thread::sleep(Duration::from_secs(5).min(remaining(deadline)?));
            }
            Some("failed") => return Err("PaddleOCR 任务解析失败，请检查图片和模型。".into()),
            Some("done") => {
                let url = data
                    .pointer("/resultUrl/jsonUrl")
                    .and_then(Value::as_str)
                    .ok_or("PaddleOCR 任务缺少 JSONL 结果地址。")?;
                // 此请求不携带 bearer Token，避免把凭据发送到结果存储站点。
                let response = client
                    .get(result_url(url)?)
                    .timeout(remaining(deadline)?)
                    .send()
                    .map_err(|_| "下载 PaddleOCR 结果失败或超时。")?;
                return parse_jsonl(&super::online_ocr::read_response(response)?);
            }
            _ => return Err("PaddleOCR 返回了未知任务状态。".into()),
        }
    }
}

/// 统一通用 OCR 的文字行、VL/Structure 的 Markdown 和版面文本块，不重复拼接全局 OCR。
pub(super) fn parse_result(value: &Value) -> Result<String, String> {
    let result = value.get("result").ok_or("PaddleOCR 响应缺少识别结果。")?;
    let mut text = String::new();
    if let Some(pages) = result.get("layoutParsingResults").and_then(Value::as_array) {
        for page in pages {
            if let Some(markdown) = page.pointer("/markdown/text").and_then(Value::as_str) {
                append(&mut text, markdown)?;
            } else {
                let blocks = page
                    .pointer("/prunedResult/parsing_res_list")
                    .and_then(Value::as_array)
                    .ok_or("PaddleOCR 版面结果缺少文字。")?;
                for block in blocks {
                    append(
                        &mut text,
                        block
                            .get("block_content")
                            .and_then(Value::as_str)
                            .ok_or("PaddleOCR 文字字段无效。")?,
                    )?;
                }
            }
        }
    } else if let Some(pages) = result.get("ocrResults").and_then(Value::as_array) {
        for page in pages {
            let lines = page
                .pointer("/prunedResult/rec_texts")
                .and_then(Value::as_array)
                .ok_or("PaddleOCR 识别结果缺少文字行。")?;
            for line in lines {
                append(&mut text, line.as_str().ok_or("PaddleOCR 文字字段无效。")?)?;
            }
        }
    } else {
        return Err("PaddleOCR 返回了不支持的结果格式。".into());
    }
    Ok(text)
}

fn append(output: &mut String, value: &str) -> Result<(), String> {
    if value.is_empty() {
        return Ok(());
    }
    let separator = usize::from(!output.is_empty());
    if output.len() + value.len() + separator > MAX_TEXT {
        return Err("识别文字超过 1 MiB。".into());
    }
    if separator != 0 {
        output.push('\n');
    }
    output.push_str(value);
    Ok(())
}

fn parse_jsonl(bytes: &[u8]) -> Result<String, String> {
    let input = std::str::from_utf8(bytes).map_err(|_| "PaddleOCR JSONL 结果编码无效。")?;
    let mut output = String::new();
    let mut found = false;
    for line in input
        .trim_start_matches('\u{feff}')
        .lines()
        .filter(|line| !line.trim().is_empty())
    {
        found = true;
        let value: Value =
            serde_json::from_str(line).map_err(|_| "PaddleOCR JSONL 结果格式无效。")?;
        append(&mut output, &parse_result(&value)?)?;
    }
    if !found {
        return Err("PaddleOCR 返回了空的 JSONL 结果。".into());
    }
    Ok(output)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn jobs_models_and_upload_contract() {
        let endpoint = "https://paddleocr.aistudio-app.com/api/v2/ocr/jobs";
        for model in ["PP-OCRv6", "PaddleOCR-VL-1.6", "PP-StructureV3"] {
            assert!(supports_model(endpoint, model));
        }
        assert!(!supports_model("https://example.com/ocr", "PP-StructureV3"));
        let (content_type, body) = multipart(b"binary-image", "PP-OCRv6");
        let boundary = content_type.split("boundary=").nth(1).unwrap();
        let body = String::from_utf8(body).unwrap();
        assert!(body.contains("name=\"model\"\r\n\r\nPP-OCRv6"));
        assert!(body.contains("name=\"optionalPayload\""));
        assert!(body.contains("filename=\"screenshot.png\""));
        assert!(body.contains("binary-image"));
        assert!(body.ends_with(&format!("--{boundary}--\r\n")));
        assert!(job_url(endpoint, "../other").is_err());
        assert!(result_url("https://files.example.com/result.jsonl?signature=x").is_ok());
        assert!(result_url("http://files.example.com/result.jsonl").is_err());
        assert!(job_data(br#"{"code":10007,"msg":"private"}"#)
            .unwrap_err()
            .find("private")
            .is_none());
    }
    #[test]
    fn reads_all_models_and_rejects_partial_or_oversize_results() {
        assert_eq!(
            parse_jsonl(
                br#"{"result":{"ocrResults":[{"prunedResult":{"rec_texts":["one","two"]}}]}}"#
            )
            .unwrap(),
            "one\ntwo"
        );
        assert_eq!(parse_result(&json!({"result":{"layoutParsingResults":[{"markdown":{"text":"# Title"},"prunedResult":{"parsing_res_list":[{"block_content":"duplicate"}]}}]}})).unwrap(), "# Title");
        assert_eq!(parse_result(&json!({"result":{"layoutParsingResults":[{"prunedResult":{"parsing_res_list":[{"block_content":"title"},{"block_content":"body"}]}}]}})).unwrap(), "title\nbody");
        assert!(parse_jsonl(b"\n").is_err());
        assert!(parse_jsonl(b"not json").is_err());
        assert!(parse_result(&json!({"result":{"layoutParsingResults":[{"markdown":{"text":"x".repeat(MAX_TEXT + 1)}}]}})).is_err());
    }
}
