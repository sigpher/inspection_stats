//! 按 config.toml 的 search 词扫描下载目录，结果写入 result.db（SQLite）。
//! 表格(xlsx/xls)→所在行；PDF→所在页；Word(doc/docx)→文本块/段落（无分页信息）。

use std::fs;
use std::io::{Cursor, Read};
use std::path::{Path, PathBuf};

use calamine::{Data, Reader, Xls, Xlsx};
use quick_xml::Reader as XmlReader;
use quick_xml::events::Event;
use rusqlite::Connection;

use crate::docbin;
use crate::ocr;
use crate::{debug, error, info};
use pdf_inspector::PdfType;

pub const RESULT_DB: &str = "result.db";

struct Hit {
    file: String,
    term: String,
    loc: String,
    snippet: String,
}

struct Scanned {
    file: String,
    broken: bool,
}

pub fn run(terms: &[String], dir: &Path, lang: &str) {
    let files = list_files(dir);
    debug!(
        "[扫描顺序] {:?}",
        files
            .iter()
            .map(|f| f.display().to_string())
            .collect::<Vec<_>>()
    );
    if files.is_empty() {
        info!("[搜索] {} 下没有可扫描的文件", dir.display());
        return;
    }
    let mut hits: Vec<Hit> = Vec::new();
    let mut scanned: Vec<Scanned> = Vec::new();
    for f in &files {
        debug!("[扫描] {}", f.display());
        let r = scan_file(f, terms, lang);
        scanned.push(Scanned {
            file: f.display().to_string(),
            broken: !r.ok,
        });
        hits.extend(r.hits);
    }
    let folder = dir
        .file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default();
    if let Err(e) = write_db(&folder, &hits, &scanned) {
        error!("[搜索] 写入 {RESULT_DB} 失败: {e}");
    }
    let ok = scanned.iter().filter(|s| !s.broken).count();
    info!(
        "[搜索] 扫描 {}/{} 个文件，命中 {} 条，结果写入 {RESULT_DB}",
        ok,
        files.len(),
        hits.len()
    );
    for term in terms {
        let n = hits.iter().filter(|h| &h.term == term).count();
        info!("[搜索] 「{term}」 命中 {n} 处");
    }
}

struct Out {
    ok: bool,
    hits: Vec<Hit>,
}

fn scan_file(path: &Path, terms: &[String], lang: &str) -> Out {
    let bytes = match fs::read(path) {
        Ok(b) => b,
        Err(e) => {
            error!("  [搜索] 读取失败 {}: {e}", path.display());
            return Out {
                ok: false,
                hits: vec![],
            };
        }
    };
    let fname = file_name(path);
    let fname = &fname;
    let kind = kind_of(&bytes, path);
    let mut hits = Vec::new();
    let ok = match kind {
        Kind::Pdf => match pdf_text_or_ocr(path, &bytes, lang) {
            Ok(pages) => {
                for term in terms {
                    for (i, page) in pages.iter().enumerate() {
                        if let Some(snip) = snippet(page, term) {
                            hits.push(Hit {
                                file: fname.clone(),
                                term: term.clone(),
                                loc: format!("第{}页", i + 1),
                                snippet: snip,
                            });
                        }
                    }
                }
                true
            }
            Err(e) => {
                error!("  [搜索] {}: {e}", path.display());
                false
            }
        },
        Kind::Table => {
            let rows = excel_rows(path);
            match rows {
                Some(rows) => {
                    for term in terms {
                        for (sheet, row, text) in &rows {
                            if let Some(snip) = snippet(text, term) {
                                hits.push(Hit {
                                    file: fname.clone(),
                                    term: term.clone(),
                                    loc: format!("第{row}行 (工作表: {sheet})"),
                                    snippet: snip,
                                });
                            }
                        }
                    }
                    true
                }
                None => false,
            }
        }
        Kind::Docx => {
            let paras = docx_paragraphs(&bytes);
            match paras {
                Ok(paras) => {
                    for term in terms {
                        for (i, p) in paras.iter().enumerate() {
                            if let Some(snip) = snippet(p, term) {
                                hits.push(Hit {
                                    file: fname.clone(),
                                    term: term.clone(),
                                    loc: format!("第{}段", i + 1),
                                    snippet: snip,
                                });
                            }
                        }
                    }
                    true
                }
                Err(e) => {
                    error!("  [搜索] docx 解析失败 {}: {e}", path.display());
                    false
                }
            }
        }
        Kind::DocBin => {
            let text = docbin::doc_text(&bytes);
            match text {
                Some(text) => {
                    for term in terms {
                        if let Some(snip) = snippet(&text, term) {
                            hits.push(Hit {
                                file: fname.clone(),
                                term: term.clone(),
                                loc: "全文".to_string(),
                                snippet: snip,
                            });
                        }
                    }
                    true
                }
                None => {
                    error!(
                        "  [搜索] .doc 无法解析 {}: 未找到 WordDocument 流",
                        path.display()
                    );
                    false
                }
            }
        }
        Kind::Other => {
            for term in terms {
                let text = crate::http::decode_text(&bytes);
                if let Some(snip) = snippet(&text, term) {
                    hits.push(Hit {
                        file: fname.clone(),
                        term: term.clone(),
                        loc: "文本".to_string(),
                        snippet: snip,
                    });
                }
            }
            true
        }
    };
    Out { ok, hits }
}

enum Kind {
    Pdf,
    Table,
    Docx,
    DocBin,
    Other,
}

fn kind_of(bytes: &[u8], path: &Path) -> Kind {
    if bytes.starts_with(b"%PDF") {
        return Kind::Pdf;
    }
    if bytes.starts_with(b"PK\x03\x04") {
        if let Ok(z) = zip::ZipArchive::new(Cursor::new(bytes)) {
            let mut names = z.file_names();
            if names.any(|n| n == "word/document.xml") {
                return Kind::Docx;
            }
            if names.any(|n| n == "xl/workbook.xml") {
                return Kind::Table;
            }
        }
        return Kind::Other;
    }
    if bytes.starts_with(b"\xD0\xCF\x11\xE0\xA1\xB1\x1A\xE1") {
        let ext = path
            .extension()
            .and_then(|e| e.to_str())
            .unwrap_or("")
            .to_lowercase();
        if ext == "xls" || ext == "et" || calamine::open_workbook::<Xls<_>, _>(path).is_ok() {
            return Kind::Table;
        }
        return Kind::DocBin;
    }
    Kind::Other
}

/// 用 pdf-inspector 快速判定 PDF 是否扫描件/图片型：
/// - Some(true)：Scanned / ImageBased，直接走 OCR；
/// - None：TextBased / Mixed / 检测失败，先按文本提取，文本极少再回退 OCR。
///
/// 检测在独立线程中带 30s 超时，避免畸形 PDF 卡死。
fn pdf_needs_ocr(bytes: &[u8]) -> Option<bool> {
    let owned = bytes.to_vec();
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let r = pdf_inspector::detect_pdf_mem(&owned)
            .ok()
            .map(|info| matches!(info.pdf_type, PdfType::Scanned | PdfType::ImageBased));
        let _ = tx.send(r);
    });
    rx.recv_timeout(std::time::Duration::from_secs(30))
        .unwrap_or_default()
}

/// 优先用 pdf-extract 取文本；pdf-inspector 判定为扫描件时直接 OCR；
/// 文本提取失败或文本极少（图片型/混合）时回退到 OCR。
fn pdf_text_or_ocr(path: &Path, bytes: &[u8], lang: &str) -> Result<Vec<String>, String> {
    let force_ocr = pdf_needs_ocr(bytes);
    match pdf_pages(bytes) {
        Ok(pages) => {
            let use_ocr = force_ocr.unwrap_or_else(|| pdf_text_sparse(&pages));
            if use_ocr {
                info!(
                    "[OCR] {} 判定需 OCR（扫描件/文本极少），尝试识别",
                    path.display()
                );
                match ocr::ocr_pdf(path, lang) {
                    Ok(op) if !pdf_text_sparse(&op) => Ok(op),
                    Ok(_) => Err(
                        "OCR 未识别出文本（可能缺少对应语言包，或页面纯为图片/空白）".to_string(),
                    ),
                    Err(e) => Err(e),
                }
            } else {
                Ok(pages)
            }
        }
        Err(e) => {
            info!("[OCR] {} 文本提取失败({e})，尝试 OCR", path.display());
            ocr::ocr_pdf(path, lang)
        }
    }
}

/// 所有页非空白字符总数低于阈值，视为扫描件/图片型 PDF。
fn pdf_text_sparse(pages: &[String]) -> bool {
    let total: usize = pages
        .iter()
        .map(|p| p.chars().filter(|c| !c.is_whitespace()).count())
        .sum();
    total < 50
}

fn pdf_pages(bytes: &[u8]) -> Result<Vec<String>, String> {
    let (tx, rx) = std::sync::mpsc::channel();
    let owned = bytes.to_vec();
    std::thread::spawn(move || {
        let _ = tx.send(pdf_extract::extract_text_from_mem_by_pages(&owned));
    });
    match rx.recv_timeout(std::time::Duration::from_secs(120)) {
        Ok(Ok(pages)) => Ok(pages),
        Ok(Err(e)) => Err(format!("PDF 文本提取失败: {e}")),
        Err(_) => Err("PDF 文本提取超时(>120s, 可能为超大/扫描型文件)".to_string()),
    }
}

fn excel_rows(path: &Path) -> Option<Vec<(String, usize, String)>> {
    let ext = path
        .extension()
        .and_then(|e| e.to_str())
        .unwrap_or("")
        .to_lowercase();
    if ext == "xls" || ext == "et" {
        let mut wb = calamine::open_workbook::<Xls<_>, _>(path).ok()?;
        scan_excel(&mut wb)
    } else {
        let mut wb = calamine::open_workbook::<Xlsx<_>, _>(path).ok()?;
        scan_excel(&mut wb)
    }
}

fn scan_excel<R>(wb: &mut R) -> Option<Vec<(String, usize, String)>>
where
    R: Reader<std::io::BufReader<std::fs::File>>,
{
    let names: Vec<String> = wb.sheet_names();
    let mut rows: Vec<(String, usize, String)> = Vec::new();
    for name in names {
        let rng = wb.worksheet_range(&name).ok()?;
        for (i, row) in rng.rows().enumerate() {
            let text: String = row.iter().map(cell_str).collect::<Vec<_>>().join(" ");
            if !text.trim().is_empty() {
                rows.push((name.clone(), i + 1, text));
            }
        }
    }
    Some(rows)
}

fn cell_str(c: &Data) -> String {
    match c {
        Data::String(s) => s.clone(),
        Data::Int(i) => i.to_string(),
        Data::Float(f) => {
            if f.fract() == 0.0 {
                format!("{}", *f as i64)
            } else {
                format!("{f}")
            }
        }
        Data::Bool(b) => b.to_string(),
        Data::DateTime(d) => format!("{d}"),
        _ => String::new(),
    }
}

fn docx_paragraphs(bytes: &[u8]) -> Result<Vec<String>, String> {
    let mut z = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|e| e.to_string())?;
    let mut xml = String::new();
    z.by_name("word/document.xml")
        .map_err(|e| e.to_string())?
        .read_to_string(&mut xml)
        .map_err(|e| e.to_string())?;
    let mut reader = XmlReader::from_str(&xml);
    reader.config_mut().trim_text(true);
    let mut paras: Vec<String> = Vec::new();
    let mut in_p = false;
    let mut buf = String::new();
    loop {
        match reader.read_event() {
            Ok(Event::Start(ref e)) => {
                if e.local_name().as_ref() == b"p" {
                    in_p = true;
                    buf = String::new();
                }
            }
            Ok(Event::Text(ref t)) => {
                if in_p {
                    buf.push_str(&t.unescape().map_err(|e| e.to_string())?);
                }
            }
            Ok(Event::CData(ref t)) => {
                if in_p {
                    let s = std::str::from_utf8(t.as_ref()).unwrap_or("");
                    buf.push_str(s);
                }
            }
            Ok(Event::End(ref e)) => {
                if e.local_name().as_ref() == b"p" {
                    if !buf.trim().is_empty() {
                        paras.push(buf.trim().to_string());
                    }
                    in_p = false;
                }
            }
            Ok(Event::Eof) => break,
            Err(e) => return Err(e.to_string()),
            _ => {}
        }
    }
    Ok(paras)
}

fn snippet(text: &str, term: &str) -> Option<String> {
    // OCR 输出常在汉字间插入空格（如“天 地 壹 号”），且英文可能有空格；
    // 检索时去掉两端空白再匹配，避免扫描件漏检。
    let ntext: String = text.chars().filter(|c| !c.is_whitespace()).collect();
    let nterm: String = term.chars().filter(|c| !c.is_whitespace()).collect();
    let low_text = ntext.to_lowercase();
    let low_term = nterm.to_lowercase();
    let idx = low_text.find(&low_term)?;
    let c_idx = low_text[..idx].chars().count();
    let c_total = low_text.chars().count();
    let c_start = c_idx.saturating_sub(15);
    let c_end = (c_idx + low_term.chars().count() + 20).min(c_total);
    let mut s: String = ntext.chars().skip(c_start).take(c_end - c_start).collect();
    if s.chars().count() > 60 {
        s = s.chars().take(60).collect::<String>() + "…";
    }
    Some(format!("“…{s}…”"))
}

fn list_files(dir: &Path) -> Vec<PathBuf> {
    let mut v = Vec::new();
    if let Ok(rd) = fs::read_dir(dir) {
        for e in rd.flatten() {
            if e.path().is_file() {
                v.push(e.path());
            }
        }
    }
    v.sort();
    v
}

fn file_name(p: &Path) -> String {
    p.file_name()
        .map(|s| s.to_string_lossy().to_string())
        .unwrap_or_default()
}

fn write_db(folder: &str, hits: &[Hit], scanned: &[Scanned]) -> Result<(), String> {
    let _ = fs::remove_file(RESULT_DB);
    let conn = Connection::open(RESULT_DB).map_err(|e| e.to_string())?;
    conn.execute_batch(
        "CREATE TABLE results (\
            id INTEGER PRIMARY KEY,\
            term TEXT NOT NULL,\
            file TEXT NOT NULL,\
            loc TEXT NOT NULL,\
            snippet TEXT NOT NULL,\
            folder TEXT NOT NULL,\
            run_at TEXT NOT NULL\
        );\
        CREATE TABLE unparsed (\
            id INTEGER PRIMARY KEY,\
            file TEXT NOT NULL,\
            folder TEXT NOT NULL,\
            run_at TEXT NOT NULL\
        );",
    )
    .map_err(|e| e.to_string())?;

    let now = chrono_now();
    let mut ins = conn
        .prepare(
            "INSERT INTO results (term,file,loc,snippet,folder,run_at) VALUES (?1,?2,?3,?4,?5,?6)",
        )
        .map_err(|e| e.to_string())?;
    for h in hits {
        ins.execute((&h.term, &h.file, &h.loc, &h.snippet, folder, &now))
            .map_err(|e| e.to_string())?;
    }
    let mut unp = conn
        .prepare("INSERT INTO unparsed (file,folder,run_at) VALUES (?1,?2,?3)")
        .map_err(|e| e.to_string())?;
    for s in scanned.iter().filter(|s| s.broken) {
        unp.execute((&s.file, folder, &now))
            .map_err(|e| e.to_string())?;
    }
    Ok(())
}

// 无 chrono 依赖，直接用格式化时间
fn chrono_now() -> String {
    use std::time::{SystemTime, UNIX_EPOCH};
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs();
    let days = secs / 86_400;
    let (y, m, d) = crate::time::civil_from_days(days as i64);
    let rem = secs % 86_400;
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}
