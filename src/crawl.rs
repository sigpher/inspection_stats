use std::fs;
use std::path::Path;
use std::sync::Mutex;

use regex::Regex;
use serde_json::Value;

use crate::html::{self, Article, Kind};
use crate::http;
use crate::{error, info};

static NAME_LOCK: Mutex<()> = Mutex::new(());

pub async fn run_region(
    client: &reqwest::Client,
    seed: &str,
    region: &str,
    month: u32,
    year: i64,
    dir: &Path,
) -> usize {
    if seed.contains("amr.jiangxi.gov.cn") {
        crawl_jiangxi(client, seed, region, month, year, dir).await
    } else if seed.contains("amr.hunan.gov.cn") || seed.contains("scjgj.fujian.gov.cn") {
        crawl_datepath(client, seed, region, month, year, dir).await
    } else {
        crawl_trs(client, seed, region, month, year, dir).await
    }
}

async fn crawl_trs(
    client: &reqwest::Client,
    seed: &str,
    region: &str,
    month: u32,
    year: i64,
    dir: &Path,
) -> usize {
    let html = match http::fetch_html(client, seed).await {
        Ok(h) => h,
        Err(e) => {
            error!("[{region}] 列表页请求失败: {e}");
            return 0;
        }
    };
    let seg = Regex::new(&format!("/{year}{month:02}/")).unwrap();
    let t_re = Regex::new(r"t\d{4,}\.\w{3,5}").unwrap();
    let mut cands: Vec<String> = Vec::new();
    for c in html::anchors().captures_iter(&html) {
        let href = html::absolutize(seed, &c[1]);
        if href.is_empty() || cands.contains(&href) {
            continue;
        }
        let is_article = href.contains("post_")
            || href.contains("content_")
            || t_re.is_match(&href)
            || seg.is_match(&href);
        if is_article {
            cands.push(href);
        }
        if cands.len() >= 60 {
            break;
        }
    }
    let mut got = 0usize;
    for art_url in &cands {
        let html = match http::fetch_html_with(client, art_url, Some(seed)).await {
            Ok(h) => h,
            Err(_) => continue,
        };
        let a = html::parse_article(&html, art_url);
        if !article_matches(a.date.0, a.date.1, year, month) {
            continue;
        }
        got += handle_article(client, &a, region, dir, art_url).await;
    }
    got
}

async fn crawl_datepath(
    client: &reqwest::Client,
    seed: &str,
    region: &str,
    month: u32,
    year: i64,
    dir: &Path,
) -> usize {
    let html = match http::fetch_html(client, seed).await {
        Ok(h) => h,
        Err(e) => {
            error!("[{region}] 列表页请求失败: {e}");
            return 0;
        }
    };
    let token = format!("{year}{month:02}");
    let mut seen: Vec<String> = Vec::new();
    for c in html::anchors().captures_iter(&html) {
        let href = html::absolutize(seed, &c[1]);
        if href.contains(&token) && !seen.contains(&href) {
            seen.push(href);
        }
    }
    let mut got = 0usize;
    for art_url in &seen {
        let html = match http::fetch_html_with(client, art_url, Some(seed)).await {
            Ok(h) => h,
            Err(_) => continue,
        };
        let a = html::parse_article(&html, art_url);
        got += handle_article(client, &a, region, dir, art_url).await;
    }
    got
}

async fn handle_article(
    client: &reqwest::Client,
    a: &Article,
    region: &str,
    dir: &Path,
    art_url: &str,
) -> usize {
    let issue = html::extract_issue(&a.title, a.date);
    let mut got = 0usize;
    for (href, label) in &a.attachments {
        let ext = html::ext_of(href);
        if ext.is_empty() {
            continue;
        }
        match html::classify(label) {
            Kind::Unqualified => {
                info!("  [{region}] 跳过不合格文件: {label}");
            }
            Kind::Keep | Kind::Mixed => {
                if download_file(client, href, region, &issue, label, &ext, dir, art_url).await {
                    got += 1;
                }
            }
            Kind::Ignore => {
                info!("  [{region}] 跳过辅助文件: {label}");
            }
        }
    }
    got
}

#[allow(clippy::too_many_arguments)]
async fn download_file(
    client: &reqwest::Client,
    url: &str,
    region: &str,
    issue: &str,
    fname: &str,
    ext: &str,
    dir: &Path,
    referer: &str,
) -> bool {
    let label = sanitize(fname);
    let mut base = if label.to_lowercase().ends_with(&format!(".{ext}")) {
        label
    } else {
        format!("{label}.{ext}")
    };
    if let Some((s, e)) = base.rsplit_once('.')
        && s.ends_with(char::is_whitespace)
    {
        base = format!("{}.{}", s.trim_end(), e);
    }
    let (stem, suffix) = match base.rsplit_once('.') {
        Some((s, e)) if !s.is_empty() => (s.to_string(), format!(".{e}")),
        _ => (base.clone(), String::new()),
    };
    let bytes = match http::fetch_bytes_with(client, url, Some(referer)).await {
        Ok(b) if !b.is_empty() => b,
        Ok(_) => {
            error!("  [下载失败] 空文件 {url}");
            return false;
        }
        Err(e) => {
            error!("  [下载失败] {url} -> {e}");
            return false;
        }
    };
    let _g = NAME_LOCK.lock().unwrap();
    let mut name = format!("{region}-{issue}-{base}");
    let mut n = 1;
    while dir.join(&name).exists() {
        n += 1;
        name = format!("{region}-{issue}-{stem}-{n}{suffix}");
    }
    let target = dir.join(&name);
    match fs::write(&target, &bytes) {
        Ok(_) => {
            info!("  [下载] {}", target.display());
            true
        }
        Err(e) => {
            error!("  [写入失败] {}: {e}", target.display());
            false
        }
    }
}

fn article_matches(y: i32, m: u32, year: i64, month: u32) -> bool {
    y as i64 == year && m == month
}

fn sanitize(s: &str) -> String {
    let cleaned: String = s
        .chars()
        .filter(|c| {
            !matches!(c, '/' | '\\' | ':' | '*' | '?' | '"' | '<' | '>' | '|') && !c.is_control()
        })
        .collect();
    let t = cleaned.trim();
    if t.is_empty() {
        "文件".to_string()
    } else {
        t.to_string()
    }
}

async fn crawl_jiangxi(
    client: &reqwest::Client,
    base: &str,
    region: &str,
    month: u32,
    year: i64,
    dir: &Path,
) -> usize {
    let (scheme, after) = base.split_once("://").unwrap();
    let host = after.split('/').next().unwrap_or(after);
    let api = format!("{scheme}://{host}/queryList");
    let mut got = 0usize;
    for page in 1..=10u32 {
        if page > 1 {
            http::jitter_sleep(300, 900).await;
        }
        let body = format!(
            "current={page}&unitid=368486&webSiteCode=amr&channelCode=spcjxx&perPage=20&titleMax=34"
        );
        let resp = match client
            .post(&api)
            .header("Content-Type", "application/x-www-form-urlencoded")
            .header("User-Agent", http::user_agent())
            .header("Referer", base)
            .header("Accept", "application/json, text/javascript, */*; q=0.01")
            .header("Accept-Language", "zh-CN,zh;q=0.9")
            .header("X-Requested-With", "XMLHttpRequest")
            .body(body)
            .send()
            .await
        {
            Ok(r) => r,
            Err(e) => {
                error!("[{region}] API 请求失败: {e}");
                return got;
            }
        };
        let text = match resp.text().await {
            Ok(t) => t,
            Err(e) => {
                error!("[{region}] API 读取失败: {e}");
                return got;
            }
        };
        let v: Value = match serde_json::from_str(&text) {
            Ok(v) => v,
            Err(e) => {
                error!("[{region}] API 响应解析失败: {e}");
                return got;
            }
        };
        let results = match v.pointer("/data/results").and_then(|r| r.as_array()) {
            Some(r) => r,
            None => return got,
        };
        if results.is_empty() {
            break;
        }
        let mut past_target = false;
        for item in results {
            let src = &item["source"];
            let (y0, m0, d0) = html::date_from_str(src["pubDate"].as_str().unwrap_or(""));
            if y0 == 0 {
                continue;
            }
            if (y0 as i64) < year || ((y0 as i64) == year && m0 < month) {
                past_target = true;
                break;
            }
            if (y0 as i64) != year || m0 != month {
                continue;
            }
            let title = src["title"].as_str().unwrap_or_default().to_string();
            let issue = html::extract_issue(&title, (y0, m0, d0));
            let files: Vec<Value> = src["articleFiles"]
                .as_str()
                .and_then(|s| serde_json::from_str::<Value>(s).ok())
                .and_then(|v| v.as_array().cloned())
                .unwrap_or_default();
            for f in &files {
                let fname = f["fileName"].as_str().unwrap_or_default().to_string();
                let url = format!(
                    "{}{}",
                    f["domainName"].as_str().unwrap_or_default(),
                    f["filePath"].as_str().unwrap_or_default()
                );
                let ext = html::ext_of(&url);
                if ext.is_empty() {
                    continue;
                }
                match html::classify(&fname) {
                    Kind::Unqualified => {
                        info!("  [{region}] 跳过不合格文件: {fname}");
                    }
                    Kind::Keep | Kind::Mixed => {
                        if download_file(client, &url, region, &issue, &fname, &ext, dir, base)
                            .await
                        {
                            got += 1;
                        }
                    }
                    Kind::Ignore => {
                        info!("  [{region}] 跳过辅助文件: {fname}");
                    }
                }
            }
        }
        if past_target {
            break;
        }
    }
    got
}
