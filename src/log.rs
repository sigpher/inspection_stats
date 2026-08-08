//! 文件 + 终端双写日志：带时间戳的行追加写入 logs/{month}.log，同时原样输出到终端。
//! 日志目录在下载目录之外，搜索扫描不会误扫日志文件。
//! 终端输出带 emoji + ANSI 颜色区分级别；写入文件的行保持纯文本（无转义码）。

use std::fs::{self, File, OpenOptions};
use std::io::{self, IsTerminal, Write};
use std::sync::Mutex;

static HANDLE: Mutex<Option<File>> = Mutex::new(None);

const GREEN: &str = "\x1b[32m";
const YELLOW: &str = "\x1b[33m";
const RED: &str = "\x1b[31m";
const CYAN: &str = "\x1b[36m";
const DIM: &str = "\x1b[2m";
const BOLD: &str = "\x1b[1m";
const RESET: &str = "\x1b[0m";

const EMOJI_INFO: &str = "\u{2139}\u{FE0F} "; // ℹ️
const EMOJI_WARN: &str = "\u{26A0}\u{FE0F} "; // ⚠️
const EMOJI_ERROR: &str = "\u{274C} "; // ❌
const EMOJI_DEBUG: &str = "\u{1F50D} "; // 🔍
const EMOJI_DONE: &str = "\u{2705} "; // ✅（用于 [完成]/[搜索] 等关键节点）

fn level_style(level: &str) -> (&'static str, &'static str) {
    match level {
        "INFO" => (EMOJI_INFO, GREEN),
        "WARN" => (EMOJI_WARN, YELLOW),
        "ERROR" => (EMOJI_ERROR, RED),
        "DEBUG" => (EMOJI_DEBUG, CYAN),
        "DONE" => (EMOJI_DONE, BOLD),
        _ => ("", GREEN),
    }
}

pub fn init(folder: &str) {
    fs::create_dir_all("logs").expect("无法创建 logs 目录");
    let path = format!("logs/{folder}.log");
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .unwrap_or_else(|e| panic!("无法打开日志文件 {path}: {e}"));
    *HANDLE.lock().unwrap() = Some(file);
}

fn timestamp() -> String {
    let secs = crate::time::local_secs();
    let days = secs / 86_400;
    let (y, m, d) = crate::time::civil_from_days(days);
    let rem = secs % 86_400;
    format!(
        "{y:04}-{m:02}-{d:02} {:02}:{:02}:{:02}",
        rem / 3600,
        (rem % 3600) / 60,
        rem % 60
    )
}

pub fn log(level: &str, to_stderr: bool, msg: &str) {
    let ts = timestamp();
    let line = format!("{ts} [{level}] {msg}");
    if let Some(f) = HANDLE.lock().unwrap().as_mut() {
        let _ = writeln!(f, "{line}");
    }
    let tty = if to_stderr {
        io::stderr().is_terminal()
    } else {
        io::stdout().is_terminal()
    };
    if tty {
        let (emoji, color) = level_style(level);
        let rendered = format!("{DIM}{ts}{RESET} {BOLD}{color}[{emoji}{level}]{RESET} {msg}");
        if to_stderr {
            eprintln!("{rendered}");
        } else {
            println!("{rendered}");
        }
    } else if to_stderr {
        eprintln!("{line}");
    } else {
        println!("{line}");
    }
}

/// 常规进展，控制台 println! 输出
#[macro_export]
macro_rules! info {
    ($($arg:tt)*) => {
        $crate::log::log("INFO", false, &format!($($arg)*))
    };
}

/// 完成/里程碑，控制台 println! 输出（✅ 加粗）
#[macro_export]
macro_rules! done {
    ($($arg:tt)*) => {
        $crate::log::log("DONE", false, &format!($($arg)*))
    };
}

/// 警告，控制台 println! 输出
#[macro_export]
macro_rules! warn {
    ($($arg:tt)*) => {
        $crate::log::log("WARN", false, &format!($($arg)*))
    };
}

/// 错误，控制台 eprintln! 输出
#[macro_export]
macro_rules! error {
    ($($arg:tt)*) => {
        $crate::log::log("ERROR", true, &format!($($arg)*))
    };
}

/// 调试信息，仅当 SW_DEBUG 环境变量存在时输出
#[macro_export]
macro_rules! debug {
    ($($arg:tt)*) => {{
        if std::env::var("SW_DEBUG").is_ok() {
            $crate::log::log("DEBUG", true, &format!($($arg)*));
        }
    }};
}
