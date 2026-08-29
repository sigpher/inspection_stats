use std::fs;

use regex::Regex;

pub const REGIONS: &[(&str, &str)] = &[
    ("amr.hunan.gov.cn", "湖南"),
    ("scjg.hubei.gov.cn", "湖北"),
    ("amr.gd.gov.cn", "广东"),
    ("scjgj.fujian.gov.cn", "福建"),
    ("scjdglj.gxzf.gov.cn", "广西"),
    ("amr.jiangxi.gov.cn", "江西"),
    ("www.gz.gov.cn", "广州"),
    ("amr.sz.gov.cn", "深圳"),
    ("www.jiangmen.gov.cn", "江门"),
    ("www.zs.gov.cn", "中山"),
    ("fsamr.foshan.gov.cn", "佛山"),
    ("scjgj.hechi.gov.cn", "河池"),
    ("www.zhuhai.gov.cn", "珠海"),
];

pub fn month_from_config() -> u32 {
    let t = fs::read_to_string("config.toml").expect("无法读取 config.toml");
    let re = Regex::new(r#"month\s*=\s*"?(\d{1,2})"#).unwrap();
    re.captures(&t)
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse().ok())
        .unwrap_or_else(|| panic!("config.toml 中没有找到 month 配置"))
}

/// Umi-OCR 开放 API 地址（默认 http://127.0.0.1:1224）。需在 Umi-OCR“全局设置”启用“开放API接口服务”。
pub fn umi_ocr_url() -> String {
    let t = fs::read_to_string("config.toml").expect("无法读取 config.toml");
    let re = Regex::new(r#"umi_ocr_url\s*=\s*"([^"]*)""#).unwrap();
    re.captures(&t)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
        .unwrap_or_else(|| "http://127.0.0.1:1224".to_string())
}

/// 全部 Umi-OCR 实例地址（负载均衡用）。优先读 `umi_ocr_urls = ["...", "..."]`；
/// 若未配置则退化为单个 `umi_ocr_url`（默认 http://127.0.0.1:1224）。
pub fn umi_ocr_instances() -> Vec<String> {
    let t = fs::read_to_string("config.toml").expect("无法读取 config.toml");
    if let Some(c) = Regex::new(r"(?s)umi_ocr_urls\s*=\s*\[(.*?)\]").unwrap().captures(&t) {
        let v: Vec<String> = c[1]
            .split([',', '，', ';', '；'])
            .map(|s| s.trim().trim_matches(['"', '\'', ' ']).to_string())
            .filter(|s| !s.is_empty())
            .collect();
        if !v.is_empty() {
            return v;
        }
    }
    let single = umi_ocr_url();
    if single.is_empty() {
        vec!["http://127.0.0.1:1224".to_string()]
    } else {
        vec![single]
    }
}

/// OCR 光栅化分辨率（DPI），默认 150。越低越快但精度略降；扫描件文字清晰时可调低提速。
pub fn ocr_dpi() -> u32 {
    let t = fs::read_to_string("config.toml").expect("无法读取 config.toml");
    let re = Regex::new(r#"ocr_dpi\s*=\s*(\d{1,3})"#).unwrap();
    re.captures(&t)
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse().ok())
        .unwrap_or(150)
}

pub fn search_terms() -> Vec<String> {
    let t = fs::read_to_string("config.toml").expect("无法读取 config.toml");
    let re = Regex::new(r"(?s)search\s*=\s*\[(.*?)\]").unwrap();
    let Some(c) = re.captures(&t) else {
        return Vec::new();
    };
    c[1].split([',', '，', ';', '；'])
        .map(|s| s.trim().trim_matches(['"', '\'', ' ']).to_string())
        .filter(|s| !s.is_empty())
        .collect()
}

fn read() -> String {
    fs::read_to_string("config.toml").expect("无法读取 config.toml")
}

/// 单页 OCR 的 HTTP 请求超时（秒），默认 60。
pub fn ocr_page_timeout() -> u64 {
    let t = read();
    Regex::new(r#"ocr_page_timeout\s*=\s*(\d{1,4})"#)
        .unwrap()
        .captures(&t)
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse().ok())
        .unwrap_or(60)
}

/// 单个 PDF 整体 OCR 超时（秒），默认 600；超时则该文件标“无法解析”。
pub fn ocr_pdf_timeout() -> u64 {
    let t = read();
    Regex::new(r#"ocr_pdf_timeout\s*=\s*(\d{1,5})"#)
        .unwrap()
        .captures(&t)
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse().ok())
        .unwrap_or(600)
}

/// PDF 文本提取（pdf-extract）超时（秒），默认 120。
pub fn pdf_extract_timeout() -> u64 {
    let t = read();
    Regex::new(r#"pdf_extract_timeout\s*=\s*(\d{1,4})"#)
        .unwrap()
        .captures(&t)
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse().ok())
        .unwrap_or(120)
}

/// Umi-OCR 使用的语言模型（API options 里的 ocr.language），默认 models/config_chinese.txt。
pub fn ocr_language() -> String {
    let t = read();
    Regex::new(r#"ocr_language\s*=\s*"([^"]*)""#)
        .unwrap()
        .captures(&t)
        .and_then(|c| c.get(1))
        .map(|m| m.as_str().to_string())
        .unwrap_or_else(|| "models/config_chinese.txt".to_string())
}

/// 命中上下文片段的总字符窗口（约）；默认 180，取命中词前 1/3、后 2/3。
pub fn snippet_len() -> usize {
    let t = read();
    Regex::new(r#"snippet_len\s*=\s*(\d{2,4})"#)
        .unwrap()
        .captures(&t)
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse().ok())
        .unwrap_or(180)
}

/// PDF 文本非空白字符总数低于该阈值则视为扫描件 / 图片型，默认 50。
pub fn sparse_threshold() -> usize {
    let t = read();
    Regex::new(r#"sparse_threshold\s*=\s*(\d{1,4})"#)
        .unwrap()
        .captures(&t)
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse().ok())
        .unwrap_or(50)
}

/// 本地时区相对 UTC 的偏移秒数（东八区 = +8h = 28800），默认 28800。
pub fn tz_offset() -> i64 {
    let t = read();
    Regex::new(r#"tz_offset\s*=\s*(-?\d{1,6})"#)
        .unwrap()
        .captures(&t)
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse().ok())
        .unwrap_or(28_800)
}

/// HTTP 客户端请求超时（秒），默认 120。
pub fn http_timeout() -> u64 {
    let t = read();
    Regex::new(r#"http_timeout\s*=\s*(\d{1,4})"#)
        .unwrap()
        .captures(&t)
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse().ok())
        .unwrap_or(120)
}

/// HTTP 下载重试次数，默认 3。
pub fn max_retries() -> u32 {
    let t = read();
    Regex::new(r#"max_retries\s*=\s*(\d{1,3})"#)
        .unwrap()
        .captures(&t)
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse().ok())
        .unwrap_or(3)
}

/// HTTP 重试退避基准毫秒（第 n 次退避 = base << n），默认 800。
pub fn retry_backoff_base() -> u64 {
    let t = read();
    Regex::new(r#"retry_backoff_base\s*=\s*(\d{1,5})"#)
        .unwrap()
        .captures(&t)
        .and_then(|c| c.get(1))
        .and_then(|m| m.as_str().parse().ok())
        .unwrap_or(800)
}

pub fn read_sites() -> Vec<String> {
    let t = fs::read_to_string("website.md").expect("无法读取 website.md");
    t.lines()
        .map(str::trim)
        .filter(|l| l.starts_with("http"))
        .map(str::to_string)
        .collect()
}

pub fn region_of(url: &str) -> Option<&'static str> {
    REGIONS
        .iter()
        .find(|(d, _)| url.contains(d))
        .map(|(_, n)| *n)
}
