//! 扫描件 / 图片型 PDF 的 OCR：调用本地 `mineru-open-api` CLI（MinerU Open API 云端）。
//! 整篇识别 PDF，返回 markdown 全文；无分页信息，调用方按「全文」定位。
//! 依赖：已安装 mineru-open-api（`uv tool install mineru-open-api`）并配置 token
//! （`mineru-open-api auth`）。无 tesseract / Umi-OCR 回退。

use std::io;
use std::path::Path;
use std::process::Command;
use std::sync::mpsc;
use std::thread;
use std::time::Duration;

use crate::info;

/// 用 mineru-open-api 对扫描件/图片型 PDF 做 OCR，返回整篇 markdown（单元素）。
pub fn ocr_pdf(path: &Path) -> Result<Vec<String>, String> {
    let cli_timeout = crate::config::ocr_pdf_timeout();
    info!("[OCR] 调用 mineru-open-api 识别 {}", path.display());
    let p = path.to_path_buf();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let out = Command::new("mineru-open-api")
            .arg("extract")
            .arg("--ocr")
            .arg("--timeout")
            .arg(cli_timeout.to_string())
            .arg(&p)
            .output();
        let _ = tx.send(out);
    });
    let grace = Duration::from_secs(60);
    match rx.recv_timeout(Duration::from_secs(cli_timeout) + grace) {
        Ok(Ok(output)) if output.status.success() => {
            let text = clean_markdown(String::from_utf8_lossy(&output.stdout).trim());
            if text.is_empty() {
                Err("mineru-open-api 未返回文本".to_string())
            } else {
                Ok(vec![text])
            }
        }
        Ok(Ok(output)) => {
            let err = String::from_utf8_lossy(&output.stderr);
            let hint = if err.contains("token") || err.contains("auth") {
                "（未配置 token？先运行 mineru-open-api auth）"
            } else {
                ""
            };
            Err(format!("mineru-open-api 失败: {}{}", err.trim(), hint))
        }
        Ok(Err(e)) => {
            let msg = if e.kind() == io::ErrorKind::NotFound {
                "未找到 mineru-open-api，请先安装：uv tool install mineru-open-api".to_string()
            } else {
                format!("mineru-open-api 启动失败: {e}")
            };
            Err(msg)
        }
        Err(_) => Err(format!(
            "mineru-open-api OCR 超时(>{cli_timeout}s)，文件过大或网络慢"
        )),
    }
}

/// 轻微清理 OCR 输出：剥离 HTML 标签（MinerU 表格以 `<table><tr><td>` 形式内嵌在 markdown 中），
/// 表格分隔符 `|` 换成空格，便于片段阅读（匹配不受影响）。
fn clean_markdown(s: &str) -> String {
    crate::html::strip_tags(s).replace('|', " ")
}
