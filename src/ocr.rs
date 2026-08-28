//! 扫描件 / 图片型 PDF 的 OCR 解析：pdftoppm 光栅化每页，优先用本地 Umi-OCR（HTTP API）识别，
//! 不可用时回退 tesseract。只在 search.rs 文本提取失败或文本极少时由 `pdf_text_or_ocr` 调用。
//!
//! Umi-OCR 需在“全局设置”启用“开放API接口服务”（默认 http://127.0.0.1:1224）。
//! 回退路径依赖系统已安装 poppler-utils（pdftoppm）与 tesseract-ocr（含对应语言包）。

use std::collections::HashMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Arc, Condvar, Mutex, OnceLock};
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use serde_json::Value;

use crate::config;
use crate::warn;

static DPI: OnceLock<u32> = OnceLock::new();
fn dpi() -> u32 {
    *DPI.get_or_init(crate::config::ocr_dpi)
}
const PAGE_TIMEOUT: Duration = Duration::from_secs(60);

const OCR_PAGE_CONCURRENCY: usize = 4;
const OCR_MAX_CONCURRENCY: usize = 1;

static OCR_SEM: OnceLock<(Mutex<usize>, Condvar)> = OnceLock::new();
fn ocr_sem() -> &'static (Mutex<usize>, Condvar) {
    OCR_SEM.get_or_init(|| (Mutex::new(OCR_MAX_CONCURRENCY), Condvar::new()))
}
fn acquire_oci() {
    let (m, c) = ocr_sem();
    let mut g = m.lock().unwrap();
    while *g == 0 {
        g = c.wait(g).unwrap();
    }
    *g -= 1;
}
fn release_oci() {
    let (m, c) = ocr_sem();
    let mut g = m.lock().unwrap();
    *g += 1;
    c.notify_one();
}

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
    let (pngs, tmp) = rasterize(path, dpi())?;
    let result = ocr_pages_umi(&pngs);
    let _ = fs::remove_dir_all(&tmp);
    result
}

fn rasterize(path: &Path, dpi: u32) -> Result<(Vec<(usize, PathBuf)>, PathBuf), String> {
    let tmp = unique_dir();
    fs::create_dir_all(&tmp).map_err(|e| format!("OCR 临时目录创建失败: {e}"))?;
    let prefix = tmp.join("p");
    let prefix_str = prefix.to_string_lossy().to_string();
    match Command::new("pdftoppm")
        .arg("-png")
        .arg("-r")
        .arg(dpi.to_string())
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
    let mut pngs: Vec<(usize, PathBuf)> = Vec::new();
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
    Ok((pngs, tmp))
}

fn ocr_pages_umi(pngs: &[(usize, PathBuf)]) -> Result<Vec<String>, String> {
    let mut jobs: Vec<(usize, Vec<u8>)> = Vec::with_capacity(pngs.len());
    for (n, p) in pngs {
        match fs::read(p) {
            Ok(b) => jobs.push((*n, b)),
            Err(e) => return Err(format!("读取页面图像失败: {e}")),
        }
    }
    if jobs.is_empty() {
        return Ok(Vec::new());
    }
    let queue = Arc::new(Mutex::new(jobs));
    let (tx, rx) = mpsc::channel::<(usize, Result<String, String>)>();
    let ep = endpoint().to_string();
    let workers = OCR_PAGE_CONCURRENCY.min(queue.lock().unwrap().len());
    let mut handles = Vec::new();
    for _ in 0..workers {
        let queue = Arc::clone(&queue);
        let tx = tx.clone();
        let ep = ep.clone();
        handles.push(thread::spawn(move || {
            loop {
                let job = { queue.lock().unwrap().pop() };
                let Some((idx, bytes)) = job else { break };
                let res = ocr_one_page_umi(&bytes, &ep);
                let _ = tx.send((idx, res));
            }
        }));
    }
    drop(tx);
    let mut ordered: Vec<(usize, String)> = Vec::new();
    let mut first_err: Option<String> = None;
    for (idx, res) in rx {
        match res {
            Ok(t) => ordered.push((idx, t)),
            Err(e) => {
                if first_err.is_none() {
                    first_err = Some(e);
                }
            }
        }
    }
    for h in handles {
        let _ = h.join();
    }
    if let Some(e) = first_err {
        return Err(e);
    }
    ordered.sort_by_key(|(n, _)| *n);
    Ok(ordered.into_iter().map(|(_, t)| t).collect())
}

fn ocr_one_page_umi(bytes: &[u8], ep: &str) -> Result<String, String> {
    acquire_oci();
    let res = (|| {
        let b64 = B64.encode(bytes);
        let body = serde_json::json!({
            "base64": b64,
            "options": { "data.format": "text", "ocr.language": "models/config_chinese.txt" }
        });
        let client = reqwest::blocking::Client::new();
        let resp = match client
            .post(format!("{ep}/api/ocr"))
            .json(&body)
            .timeout(PAGE_TIMEOUT)
            .send()
        {
            Ok(r) => r,
            Err(e) => return Err(format!("Umi-OCR 请求失败: {e}")),
        };
        let val: Value = match resp.json() {
            Ok(v) => v,
            Err(e) => return Err(format!("Umi-OCR 响应解析失败: {e}")),
        };
        match val.get("code").and_then(|c| c.as_i64()) {
            Some(100) => Ok(val
                .get("data")
                .and_then(|d| d.as_str())
                .unwrap_or("")
                .to_string()),
            Some(101) => Ok(String::new()),
            _ => Err(format!("Umi-OCR 返回错误: {val}")),
        }
    })();
    release_oci();
    res
}

fn tesseract_ocr(path: &Path, lang: &str) -> Result<Vec<String>, String> {
    if !tesseract_has_lang(lang) {
        return Err(
            "OCR 不可用：tesseract 未安装或语言包缺失（请安装 chi_sim / eng 等）".to_string(),
        );
    }
    let (pngs, tmp) = rasterize(path, dpi())?;
    let mut texts: Vec<String> = Vec::with_capacity(pngs.len());
    for (_, p) in &pngs {
        match Command::new("tesseract")
            .arg(p)
            .arg("stdout")
            .arg("-l")
            .arg(lang)
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
                    let _ = fs::remove_dir_all(&tmp);
                    return Err("未找到 tesseract，请安装 tesseract-ocr".to_string());
                }
                warn!("[OCR] tesseract 启动失败: {e}");
                texts.push(String::new());
            }
        }
    }
    let _ = fs::remove_dir_all(&tmp);
    Ok(texts)
}

fn tesseract_has_lang(lang: &str) -> bool {
    static CACHE: OnceLock<Mutex<HashMap<String, bool>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| Mutex::new(HashMap::new()));
    if let Some(v) = cache.lock().unwrap().get(lang) {
        return *v;
    }
    let out = match Command::new("tesseract")
        .arg("--list-langs")
        .stderr(Stdio::null())
        .output()
    {
        Ok(o) => o,
        Err(_) => return false,
    };
    let ok = if !out.status.success() {
        false
    } else {
        let text = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        lang.split('+').all(|l| {
            let l = l.trim();
            !l.is_empty() && text.lines().any(|line| line.trim() == l)
        })
    };
    cache.lock().unwrap().insert(lang.to_string(), ok);
    ok
}

fn unique_dir() -> std::path::PathBuf {
    let pid = std::process::id();
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!("insp_ocr_{pid}_{n}"))
}
