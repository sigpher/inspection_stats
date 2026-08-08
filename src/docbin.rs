//! 旧版 .doc（OLE 复合文档）读取：用 `cfb` crate 定位 WordDocument 流，
//! 再以 GB18030 / UTF-16LE 双重解码取其文本块。无排版引擎，无页码信息。

use std::io::{Cursor, Read};

use cfb::CompoundFile;

/// 提取 .doc 中的可读文本块；无法识别时返回 None。
pub fn doc_text(data: &[u8]) -> Option<String> {
    let mut cf = CompoundFile::open(Cursor::new(data)).ok()?;
    let mut stream = cf.open_stream("WordDocument").ok()?;
    let mut buf = Vec::new();
    stream.read_to_end(&mut buf).ok()?;
    let blocks = text_blocks(&buf);
    if !blocks.is_empty() {
        return Some(blocks.join("\n"));
    }
    let fallback = text_blocks(data);
    if fallback.is_empty() {
        None
    } else {
        Some(fallback.join("\n"))
    }
}

fn text_blocks(bytes: &[u8]) -> Vec<String> {
    let mut blocks: Vec<String> = Vec::new();
    for s in [decode_gb(bytes), decode_utf16le(bytes)] {
        let mut cur = String::new();
        for ch in s.chars() {
            if is_texty(ch) {
                cur.push(ch);
            } else {
                if cur.chars().count() >= 4 && cur.chars().any(is_cjk) {
                    blocks.push(cur.trim().to_string());
                }
                cur = String::new();
            }
        }
        if cur.chars().count() >= 4 && cur.chars().any(is_cjk) {
            blocks.push(cur.trim().to_string());
        }
    }
    blocks.sort();
    blocks.dedup();
    blocks
}

fn decode_gb(bytes: &[u8]) -> String {
    let (cow, _, _) = encoding_rs::GB18030.decode(bytes);
    cow.into_owned()
}

fn decode_utf16le(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() / 2);
    let mut i = 0;
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
