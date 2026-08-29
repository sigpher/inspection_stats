//! 扫描件 / 图片型 PDF 的 OCR 解析：pdftoppm 光栅化每页，用本地 Umi-OCR（HTTP API）识别。
//! 只在 search.rs 文本提取失败或文本极少时由 `pdf_text_or_ocr` 调用。
//!
//! Umi-OCR 需在“全局设置”启用“开放API接口服务”。支持多个实例（`config.toml` 的 `umi_ocr_urls`），
//! 客户端用空闲实例池做负载均衡：每个实例同时只处理一个请求（Umi-OCR/Paddle 内部已多线程，
//! 并发反而因 CPU 过度订阅而变慢），多实例之间真正并行。光栅化依赖系统已安装
//! poppler-utils（pdftoppm）。

use std::collections::VecDeque;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Condvar, Mutex, OnceLock};
use std::thread;
use std::time::{Duration, Instant};

use base64::Engine;
use base64::engine::general_purpose::STANDARD as B64;
use serde_json::Value;

use crate::config;
use crate::warn;

static DPI: OnceLock<u32> = OnceLock::new();
fn dpi() -> u32 {
    *DPI.get_or_init(crate::config::ocr_dpi)
}
static PAGE_TIMEOUT: OnceLock<Duration> = OnceLock::new();
fn page_timeout() -> Duration {
    *PAGE_TIMEOUT.get_or_init(|| Duration::from_secs(config::ocr_page_timeout()))
}
static OCR_PDF_TIMEOUT: OnceLock<Duration> = OnceLock::new();
fn ocr_pdf_timeout() -> Duration {
    *OCR_PDF_TIMEOUT.get_or_init(|| Duration::from_secs(config::ocr_pdf_timeout()))
}
static OCR_LANG: OnceLock<String> = OnceLock::new();
fn ocr_lang() -> &'static str {
    OCR_LANG.get_or_init(config::ocr_language)
}

/// 单 PDF 内并发识别的 worker 数（上限由实例数决定，多余的 worker 会阻塞在实例池上）。
const OCR_PAGE_WORKERS: usize = 8;

// ── 多实例池 ────────────────────────────────────────────────────────────────
static INSTANCES: OnceLock<Vec<String>> = OnceLock::new();
fn instances() -> &'static [String] {
    INSTANCES.get_or_init(config::umi_ocr_instances).as_slice()
}

struct PoolState {
    /// 当前空闲（可分配）的实例下标
    free: VecDeque<usize>,
    /// 各实例是否仍可用（探测失败会被置 false，不再分配）
    alive: Vec<bool>,
}
static POOL: OnceLock<(Mutex<PoolState>, Condvar)> = OnceLock::new();
fn pool() -> &'static (Mutex<PoolState>, Condvar) {
    POOL.get_or_init(|| {
        let insts = instances();
        let mut alive = Vec::with_capacity(insts.len());
        let mut free = VecDeque::new();
        for (i, url) in insts.iter().enumerate() {
            let ok = reqwest::blocking::Client::new()
                .get(format!("{}/api/ocr/get_options", url))
                .timeout(Duration::from_secs(3))
                .send()
                .is_ok();
            alive.push(ok);
            if ok {
                free.push_back(i);
            } else {
                warn!("[OCR] Umi-OCR 实例不可用: {url}");
            }
        }
        (Mutex::new(PoolState { free, alive }), Condvar::new())
    })
}

/// 取一个空闲实例；若无可用实例返回 None（调用方据此判定失败/回退）。
fn acquire_instance() -> Option<usize> {
    let (m, c) = pool();
    let mut g = m.lock().unwrap();
    loop {
        if let Some(i) = g.free.pop_front() {
            return Some(i);
        }
        if g.alive.iter().any(|a| *a) {
            g = c.wait(g).unwrap();
        } else {
            return None;
        }
    }
}
fn release_instance(i: usize) {
    let (m, c) = pool();
    let mut g = m.lock().unwrap();
    if g.alive[i] {
        g.free.push_back(i);
        c.notify_one();
    }
}
fn mark_dead(i: usize) {
    let (m, _c) = pool();
    let mut g = m.lock().unwrap();
    g.alive[i] = false;
}
/// 是否至少有一个 Umi-OCR 实例可用。
fn umi_ocr_any_available() -> bool {
    pool();
    let (m, _) = pool();
    m.lock().unwrap().alive.iter().any(|a| *a)
}

/// PDF 需要 OCR 时只用本地 Umi-OCR（多实例负载均衡）；全部实例不可用则该文件无法 OCR。
pub fn ocr_pdf(path: &Path) -> Result<Vec<String>, String> {
    if !umi_ocr_any_available() {
        return Err("Umi-OCR 不可用，扫描件无法 OCR（未启用 tesseract 回退）".to_string());
    }
    umi_ocr_pdf(path)
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
    if pngs.is_empty() {
        return Ok(Vec::new());
    }
    let mut jobs: Vec<(usize, Vec<u8>)> = Vec::with_capacity(pngs.len());
    for (n, p) in pngs {
        match fs::read(p) {
            Ok(b) => jobs.push((*n, b)),
            Err(e) => return Err(format!("读取页面图像失败: {e}")),
        }
    }
    let n = jobs.len();
    let results: Vec<Mutex<Option<String>>> = (0..n).map(|_| Mutex::new(None)).collect();
    let jobs = &jobs[..];
    let results = &results[..];
    let next = AtomicUsize::new(0);
    let any_failed = AtomicBool::new(false);
    let workers = OCR_PAGE_WORKERS.min(n);
    let deadline = Instant::now().checked_add(ocr_pdf_timeout());
    thread::scope(|s| {
        for _ in 0..workers {
            s.spawn(|| {
                loop {
                    let i = next.fetch_add(1, Ordering::Relaxed);
                    if i >= n {
                        break;
                    }
                    if let Some(d) = deadline
                        && Instant::now() >= d
                    {
                        any_failed.store(true, Ordering::Relaxed);
                        break;
                    }
                    let (_, bytes) = &jobs[i];
                    let inst = match acquire_instance() {
                        Some(x) => x,
                        None => {
                            any_failed.store(true, Ordering::Relaxed);
                            continue;
                        }
                    };
                    let res = ocr_one_page_umi(bytes, inst);
                    release_instance(inst);
                    match res {
                        Ok(t) => *results[i].lock().unwrap() = Some(t),
                        Err(_) => {
                            mark_dead(inst);
                            any_failed.store(true, Ordering::Relaxed);
                        }
                    }
                }
            });
        }
    });
    if any_failed.load(Ordering::Relaxed) {
        return Err("部分页面 OCR 失败（Umi-OCR 实例不可用）".to_string());
    }
    let mut ordered: Vec<(usize, String)> = Vec::with_capacity(n);
    for (i, r) in results.iter().enumerate() {
        if let Some(t) = r.lock().unwrap().take() {
            ordered.push((jobs[i].0, t));
        }
    }
    ordered.sort_by_key(|(n, _)| *n);
    Ok(ordered.into_iter().map(|(_, t)| t).collect())
}

fn ocr_one_page_umi(bytes: &[u8], inst: usize) -> Result<String, String> {
    let ep = instances()[inst].clone();
    let b64 = B64.encode(bytes);
    let body = serde_json::json!({
        "base64": b64,
        "options": { "data.format": "text", "ocr.language": ocr_lang() }
    });
    let client = reqwest::blocking::Client::new();
    let resp = match client
        .post(format!("{ep}/api/ocr"))
        .json(&body)
        .timeout(page_timeout())
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
}

fn unique_dir() -> std::path::PathBuf {
    let pid = std::process::id();
    let n = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    std::env::temp_dir().join(format!("insp_ocr_{pid}_{n}"))
}
