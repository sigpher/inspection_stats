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
