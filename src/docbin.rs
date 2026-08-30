//! 旧版 .doc / WPS 原生 .xls（OLE 复合文档）读取：用 `cfb` crate 定位
//! WordDocument / Workbook 流，再以 GB18030 / UTF-16LE 双对齐解码取其文本块。
//! 无排版引擎，无页码信息。

use std::io::{Cursor, Read};

use cfb::CompoundFile;

/// 提取 .doc 中的可读文本块（保持文档顺序，块≈段落/行）；无法识别时返回 None。
pub fn doc_blocks(data: &[u8]) -> Option<Vec<String>> {
    if let Some(b) = cfb_blocks(data) {
        return Some(b);
    }
    let fallback = text_blocks(data);
    if fallback.is_empty() {
        None
    } else {
        Some(fallback)
    }
}

/// 提取 OLE 复合文档中常见文本流（WordDocument / Workbook / Book）的可读文本块
/// （保持文档顺序，块≈段落/行）。用于 calamine 打不开的 WPS 原生 .xls 等场景。
/// 无法识别时返回 None。
pub fn cfb_blocks(data: &[u8]) -> Option<Vec<String>> {
    let mut cf = CompoundFile::open(Cursor::new(data)).ok()?;
    let mut out: Vec<String> = Vec::new();
    for name in ["WordDocument", "Workbook", "Book"] {
        if cf.is_stream(name) {
            let mut stream = cf.open_stream(name).ok()?;
            let mut buf = Vec::new();
            stream.read_to_end(&mut buf).ok()?;
            out.extend(text_blocks(&buf));
        }
    }
    if out.is_empty() { None } else { Some(out) }
}

fn text_blocks(bytes: &[u8]) -> Vec<String> {
    // 三种解码都试，各带字节偏移：GB18030（单字节），UTF-16LE 偶/奇对齐。
    // 乱码来自“对另一种编码解码”：多为短块，真实文本是长块。
    let gb = collect_with_offs(&decode_gb(bytes), 0, 1);
    let u0 = collect_with_offs(&decode_utf16le(bytes, 0), 0, 2);
    let u1 = collect_with_offs(&decode_utf16le(bytes, 1), 1, 2);
    // 用「≥4 字块的总字数」判定主导编码，避免乱码把块号冲高。
    let score = |v: &[(usize, usize, String)]| -> usize {
        v.iter()
            .map(|(_, _, s)| s.chars().count())
            .filter(|&n| n >= 4)
            .sum()
    };
    let utf16_score = score(&u0) + score(&u1);
    let gb_score = score(&gb);
    let chosen: Vec<(usize, usize, String)> = if utf16_score > gb_score {
        let mut v = u0;
        v.extend(u1);
        v.sort_by_key(|(s, _, _)| *s);
        v
    } else {
        gb
    };
    merge_blocks(chosen)
}

/// 按字节偏移合并重叠块：错位解码与真实文本占用同一段字节，
/// 重叠时保留 CJK 字数更多的那个（真实文本更长），使块号贴近文档顺序。
fn merge_blocks(v: Vec<(usize, usize, String)>) -> Vec<String> {
    let mut kept: Vec<(usize, usize, String)> = Vec::new();
    for (s, e, text) in v {
        if let Some(last) = kept.last_mut() {
            let (ls, le, ltext) = last;
            if s < *le && *ls < e {
                let lcjk = ltext.chars().filter(|c| is_cjk(*c)).count();
                let cjk = text.chars().filter(|c| is_cjk(*c)).count();
                if cjk > lcjk {
                    *last = (s, e, text);
                }
                continue;
            }
        }
        kept.push((s, e, text));
    }
    let mut seen = std::collections::HashSet::new();
    kept.into_iter()
        .map(|(_, _, t)| t)
        .filter(|t| seen.insert(t.clone()))
        .collect()
}

fn collect_with_offs(s: &str, base: usize, stride: usize) -> Vec<(usize, usize, String)> {
    let mut out = Vec::new();
    let mut cur = String::new();
    let mut start = 0usize;
    for (i, ch) in s.chars().enumerate() {
        if is_texty(ch) {
            if cur.is_empty() {
                start = i;
            }
            cur.push(ch);
        } else if cur.chars().count() >= MIN_BLOCK && cur.chars().any(is_cjk) {
            let b = base + start * stride;
            out.push((b, b + cur.chars().count() * stride, cur.trim().to_string()));
            cur = String::new();
        } else {
            cur = String::new();
        }
    }
    if cur.chars().count() >= MIN_BLOCK && cur.chars().any(is_cjk) {
        let b = base + start * stride;
        out.push((b, b + cur.chars().count() * stride, cur.trim().to_string()));
    }
    out
}

/// 过滤掉 FIB 头/二进制区解出的短乱码块，返回“干净”块在原列表中的下标。
/// 用于定位编号（块号贴近文档段落），而不是用于关键词匹配——短的真实文本
/// （如 2 字关键词）仍需保留以命中，只是无法给出干净编号。
pub fn clean_indices(blocks: &[String]) -> Vec<usize> {
    blocks
        .iter()
        .enumerate()
        .filter(|(_, b)| {
            let n = b.chars().count();
            if n < 3 {
                return false;
            }
            let cjk = b.chars().filter(|c| is_cjk(*c)).count();
            let spaces = b.chars().filter(|c| c.is_whitespace()).count();
            cjk >= 2 && spaces * 5 <= n * 2
        })
        .map(|(i, _)| i)
        .collect()
}

/// 最短文本块长度：短于它的块丢弃（二进制数据会解出大量 1~2 个乱码汉字）。
/// 4 会把真实短关键词（如 2~3 字的商品/企业名）也滤掉导致漏检，故降至 2。
const MIN_BLOCK: usize = 2;

fn decode_gb(bytes: &[u8]) -> String {
    let (cow, _, _) = encoding_rs::GB18030.decode(bytes);
    cow.into_owned()
}

fn decode_utf16le(bytes: &[u8], byte_off: usize) -> String {
    let mut out = String::with_capacity(bytes.len() / 2);
    let mut i = byte_off;
    while i + 1 < bytes.len() {
        let c = u16::from_le_bytes([bytes[i], bytes[i + 1]]);
        if c == 0 {
            out.push(' ');
        } else if let Some(ch) = char::from_u32(c as u32) {
            out.push(ch);
        } else {
            out.push('\u{FFFD}');
        }
        i += 2;
    }
    out
}

fn is_texty(ch: char) -> bool {
    let c = ch as u32;
    (0x20..=0x7E).contains(&c)
        || (0x4E00..=0x9FFF).contains(&c)
        || (0x3000..=0x303F).contains(&c)
        || (0xFF00..=0xFFEF).contains(&c)
        || (0x2000..=0x206F).contains(&c)
}

fn is_cjk(ch: char) -> bool {
    let c = ch as u32;
    (0x4E00..=0x9FFF).contains(&c) || (0x3400..=0x4DBF).contains(&c)
}
