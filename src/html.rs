use std::sync::OnceLock;

use regex::Regex;

pub const EXTS: [&str; 9] = [
    "doc", "docx", "xls", "xlsx", "pdf", "wps", "ppt", "pptx", "zip",
];

pub struct Article {
    pub title: String,
    pub date: (i32, u32, u32),
    pub attachments: Vec<(String, String)>,
}

#[derive(PartialEq)]
pub enum Kind {
    Keep,
    Mixed,
    Unqualified,
    Ignore,
}

pub fn anchors() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| Regex::new(r#"(?is)<a[^>]*href="([^"]+)"[^>]*>(.*?)</a>"#).unwrap())
}

pub fn strip_tags(s: &str) -> String {
    let re = Regex::new(r"<[^>]+>").unwrap();
    re.replace_all(s, " ").trim().to_string()
}

pub fn absolutize(base: &str, href: &str) -> String {
    let href = href.trim().replace("&amp;", "&");
    if href.is_empty() || href.starts_with('#') || href.starts_with("javascript:") {
        return String::new();
    }
    if href.starts_with("http://") || href.starts_with("https://") {
        return href;
    }
    let (scheme, after) = match base.split_once("://") {
        Some(x) => x,
        None => return String::new(),
    };
    let (host, tail) = match after.split_once('/') {
        Some((h, t)) => (h, t),
        None => (after, ""),
    };
    let origin = format!("{scheme}://{host}");
    if href.starts_with("//") {
        return format!("{scheme}:{href}");
    }
    if href.starts_with('/') {
        return format!("{origin}{href}");
    }
    let base_path = match tail.rsplit_once('/') {
        Some((p, _)) => p.to_string(),
        None => String::new(),
    };
    let mut segs: Vec<&str> = base_path.split('/').filter(|s| !s.is_empty()).collect();
    for part in href.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                segs.pop();
            }
            _ => segs.push(part),
        }
    }
    format!("{origin}/{}", segs.join("/"))
}

pub fn scan_date(html: &str) -> (i32, u32, u32) {
    let re1 = Regex::new(r"(?iu)(PubDate|发布时间|发布日期|publishdate|createdate)[^0-9]{0,40}(20\d{2})-(\d{2})-(\d{2})").unwrap();
    if let Some(c) = re1.captures_iter(html).next() {
        return (
            c[2].parse().unwrap(),
            c[3].parse().unwrap(),
            c[4].parse().unwrap(),
        );
    }
    let re2 = Regex::new(r"\b(20\d{2})-(\d{2})-(\d{2})\b").unwrap();
    if let Some(c) = re2.captures_iter(html).next() {
        return (
            c[1].parse().unwrap(),
            c[2].parse().unwrap(),
            c[3].parse().unwrap(),
        );
    }
    (0, 0, 0)
}

pub fn date_from_str(s: &str) -> (i32, u32, u32) {
    let re = Regex::new(r"(\d{4})-(\d{2})-(\d{2})").unwrap();
    match re.captures(s) {
        Some(c) => (
            c[1].parse().unwrap(),
            c[2].parse().unwrap(),
            c[3].parse().unwrap(),
        ),
        None => (0, 0, 0),
    }
}

pub fn parse_article(html: &str, base: &str) -> Article {
    let title = Regex::new(r"(?s)<title>(.*?)</title>")
        .unwrap()
        .captures(html)
        .map(|c| c.get(1).unwrap().as_str().trim().to_string())
        .unwrap_or_default();
    let date = scan_date(html);

    let mut attachments = Vec::new();
    for c in anchors().captures_iter(html) {
        let href = absolutize(base, &c[1]);
        if href.is_empty() {
            continue;
        }
        let has_ext = EXTS.iter().any(|e| href.ends_with(&format!(".{e}")));
        if !has_ext && !href.contains("/files/") && !href.contains("/attachment/") {
            continue;
        }
        attachments.push((href, strip_tags(&c[2])));
    }
    Article {
        title,
        date,
        attachments,
    }
}

pub fn classify(name: &str) -> Kind {
    if name.contains("不合格") {
        Kind::Unqualified
    } else if name.contains("合格") {
        Kind::Keep
    } else if DATA_MARKERS.iter().any(|k| name.contains(k)) {
        Kind::Mixed
    } else {
        Kind::Ignore
    }
}

const DATA_MARKERS: [&str; 14] = [
    "附件",
    "明细",
    "汇总",
    "信息表",
    "结果",
    "通告",
    "公告",
    "抽检",
    "监测",
    "检测",
    "台账",
    "报告",
    "数据",
    "表",
];

pub fn extract_issue(title: &str, date: (i32, u32, u32)) -> String {
    let re = Regex::new(r"第\s*(\d+)\s*[期号批]").unwrap();
    if let Some(c) = re.captures(title) {
        return format!("第{}期", &c[1]);
    }
    let re8 = Regex::new(r"(20\d{6})").unwrap();
    if let Some(c) = re8.captures(title) {
        return c[1].to_string();
    }
    if date != (0, 0, 0) {
        format!("{}{:02}{:02}", date.0, date.1, date.2)
    } else {
        "期数未知".to_string()
    }
}

pub fn ext_of(url: &str) -> String {
    let clean = url.split(['?', '#']).next().unwrap_or(url);
    let ext = clean.rsplit('.').next().unwrap_or("").to_lowercase();
    if !EXTS.contains(&ext.as_str()) {
        return String::new();
    }
    ext
}
