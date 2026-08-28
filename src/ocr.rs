//! 扫描件 / 图片型 PDF 的 OCR 解析：pdftoppm 光栅化每页，优先用本地 Umi-OCR（HTTP API）识别，
//! 不可用时回退 tesseract。只在 search.rs 文本提取失败或文本极少时由 `pdf_text_or_ocr` 调用。
//!
//! Umi-OCR 需在“全局设置”启用“开放API接口服务”（默认 http://127.0.0.1:1224）。
//! 回退路径依赖系统已安装 poppler-utils（pdftoppm）与 tesseract-ocr（含对应语言包）。

use std::fs;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::OnceLock;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use serde_json::Value;

use crate::config;
use crate::warn;

const DPI: u32 = 200;
const TIMEOUT: Duration = Duration::from_secs(600);
const PAGE_TIMEOUT: Duration = Duration::from_secs(60);

static ENDPOINT: OnceLock<String> = OnceLock::new();
fn endpoint() -> &'static str {
    ENDPOINT.get_or_init(config::umi_ocr_url)
}

/// PDF 需要 OCR 时优先用 Umi-OCR；不可用或失败时回退 tesseract。
pub fn ocr_pdf(path: &Path, lang: &str) -> Result<Vec<String>, String> {
    if umi_ocr_available() {
        match umi_ocr_pdf(path) {
            Ok(t) => return Ok(t),
            Err(e) => warn!("[OCR] Umi-OCR 失败({e})，回退 tesseract"),
        }
    }
    tesseract_ocr(path, lang)
}

fn umi_ocr_available() -> bool {
    static CACHE: OnceLock<bool> = OnceLock::new();
    *CACHE.get_or_init(|| {
        reqwest::blocking::Client::new()
            .get(format!("{}/api/ocr/get_options", endpoint()))
            .timeout(Duration::from_secs(3))
            .send()
            .is_ok()
    })
}

fn umi_ocr_pdf(path: &Path) -> Result<Vec<String>, String> {
    let tmp = unique_dir();
    fs::create_dir_all(&tmp).map_err(|e| format!("OCR 临时目录创建失败: {e}"))?;
    let prefix = tmp.join("p");
    let prefix_str = prefix.to_string_lossy().to_string();

    // 1) 光栅化全部页为 PNG
    match Command::new("pdftoppm")
        .arg("-png")
        .arg("-r")
        .arg(DPI.to_string())
        .arg(path)
        .arg(&prefix_str)
        .output()
    {
        Ok(o) if o.status.success() => {}
        Ok(o) => {
            let _ = fs::remove_dir_all(&tmp);
            return Err(format!(
                "pdftoppm 执行失败: {}",
                String::from_utf8_lossy(&o.stderr)
            ));
        }
        Err(e) => {
            let _ = fs::remove_dir_all(&tmp);
            if e.kind() == std::io::ErrorKind::NotFound {
                return Err("未找到 pdftoppm，请安装 poppler-utils".to_string());
            }
            return Err(format!("pdftoppm 启动失败: {e}"));
        }
    }

    // 2) 枚举生成的页面图像（p-1.png, p-2.png ...）
    let mut pngs: Vec<(usize, std::path::PathBuf)> = Vec::new();
    if let Ok(rd) = fs::read_dir(&tmp) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if let Some(stem) = name.strip_prefix("p-").and_then(|s| s.strip_suffix(".png"))
                && let Ok(n) = stem.parse::<usize>()
            {
                pngs.push((n, e.path()));
            }
        }
    }
    pngs.sort_by_key(|(n, _)| *n);
    if pngs.is_empty() {
        let _ = fs::remove_dir_all(&tmp);
        return Err("pdftoppm 未生成任何页面图像（可能不是有效 PDF）".to_string());
    }

    // 3) 逐页 POST 到 Umi-OCR /api/ocr（base64 单图，data.format=text）
    let (tx, rx) = mpsc::channel();
    let ep = endpoint().to_string();
    thread::spawn(move || {
        let client = reqwest::blocking::Client::new();
        let mut texts: Vec<String> = Vec::new();
        let mut err: Option<String> = None;
        for (_, p) in &pngs {
            let bytes = match fs::read(p) {
                Ok(b) => b,
                Err(e) => {
                    err = Some(format!("读取页面图像失败: {e}"));
                    break;
                }
            };
            let b64 = B64.encode(&bytes);
            let body = serde_json::json!({
                "base64": b64,
                "options": { "data.format": "text", "ocr.language": "models/config_chinese.txt" }
            });
            let resp = match client
                .post(format!("{ep}/api/ocr"))
                .json(&body)
                .timeout(PAGE_TIMEOUT)
                .send()
            {
                Ok(r) => r,
                Err(e) => {
                    err = Some(format!("Umi-OCR 请求失败: {e}"));
                    break;
                }
            };
            let val: Value = match resp.json() {
                Ok(v) => v,
                Err(e) => {
                    err = Some(format!("Umi-OCR 响应解析失败: {e}"));
                    break;
                }
            };
            match val.get("code").and_then(|c| c.as_i64()) {
                Some(100) => {
                    let t = val
                        .get("data")
                        .and_then(|d| d.as_str())
                        .unwrap_or("")
                        .to_string();
                    texts.push(t);
                }
                Some(101) => texts.push(String::new()), // 该页无文字
                _ => {
                    err = Some(format!("Umi-OCR 返回错误: {val}"));
                    break;
                }
            }
        }
        let _ = tx.send(if let Some(e) = err { Err(e) } else { Ok(texts) });
    });

    let result = rx
        .recv_timeout(TIMEOUT)
        .map_err(|_| format!("Umi-OCR 超时(>{:?})，可能为超大扫描件", TIMEOUT))??;
    let _ = fs::remove_dir_all(&tmp);
    Ok(result)
}

fn tesseract_ocr(path: &Path, lang: &str) -> Result<Vec<String>, String> {
    if !tesseract_has_lang(lang) {
        return Err(
            "OCR 不可用：tesseract 未安装或语言包缺失（请安装 chi_sim / eng 等）".to_string(),
        );
    }
    let tmp = unique_dir();
    fs::create_dir_all(&tmp).map_err(|e| format!("OCR 临时目录创建失败: {e}"))?;
    let prefix = tmp.join("p");
    let prefix_str = prefix.to_string_lossy().to_string();
    match Command::new("pdftoppm")
        .arg("-png")
        .arg("-r")
        .arg(DPI.to_string())
        .arg(path)
        .arg(&prefix_str)
        .output()
    {
        Ok(o) if o.status.success() => {}
        Ok(o) => {
            let _ = fs::remove_dir_all(&tmp);
            return Err(format!(
                "pdftoppm 执行失败: {}",
                String::from_utf8_lossy(&o.stderr)
            ));
        }
        Err(e) => {
            let _ = fs::remove_dir_all(&tmp);
            if e.kind() == std::io::ErrorKind::NotFound {
                return Err("未找到 pdftoppm，请安装 poppler-utils".to_string());
            }
            return Err(format!("pdftoppm 启动失败: {e}"));
        }
    }

    let mut pngs: Vec<(usize, std::path::PathBuf)> = Vec::new();
    if let Ok(rd) = fs::read_dir(&tmp) {
        for e in rd.flatten() {
            let name = e.file_name().to_string_lossy().to_string();
            if let Some(stem) = name.strip_prefix("p-").and_then(|s| s.strip_suffix(".png"))
                && let Ok(n) = stem.parse::<usize>()
            {
                pngs.push((n, e.path()));
            }
        }
    }
    pngs.sort_by_key(|(n, _)| *n);
    if pngs.is_empty() {
        let _ = fs::remove_dir_all(&tmp);
        return Err("pdftoppm 未生成任何页面图像（可能不是有效 PDF）".to_string());
    }

    let (tx, rx) = mpsc::channel();
    let t_lang = lang.to_string();
    thread::spawn(move || {
        let mut texts: Vec<String> = Vec::new();
        for (_, p) in &pngs {
            match Command::new("tesseract")
                .arg(p)
                .arg("stdout")
                .arg("-l")
                .arg(&t_lang)
                .stderr(Stdio::null())
                .output()
            {
                Ok(o) if o.status.success() => {
                    texts.push(String::from_utf8_lossy(&o.stdout).to_string());
                }
                Ok(o) => {
                    warn!(
                        "[OCR] tesseract 单页识别失败: {}",
                        String::from_utf8_lossy(&o.stderr)
                    );
                    texts.push(String::new());
                }
                Err(e) => {
                    if e.kind() == std::io::ErrorKind::NotFound {
                        let _ = tx.send(Err("未找到 tesseract，请安装 tesseract-ocr".to_string()));
                        return;
                    }
                    warn!("[OCR] tesseract 启动失败: {e}");
                    texts.push(String::new());
                }
            }
        }
        let _ = tx.send(Ok(texts));
    });

    let result = rx
        .recv_timeout(TIMEOUT)
        .map_err(|_| format!("OCR 超时(>{:?})，可能为超大扫描件", TIMEOUT))?;
    let _ = fs::remove_dir_all(&tmp);
    result
}

fn tesseract_has_lang(lang: &str) -> bool {
    let out = match Command::new("tesseract")
        .arg("--list-langs")
        .stderr(Stdio::null())
        .output()
    {
        Ok(o) => o,
        Err(_) => return false,
    };
    if !out.status.success() {
        return false;
    }
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    lang.split('+').all(|l| {
        let l = l.trim();
        !l.is_empty() && text.lines().any(|line| line.trim() == l)
    })
}

fn unique_dir() -> std::path::PathBuf {
    let pid = std::process::id();
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!("insp_ocr_{pid}_{n}"))
}
