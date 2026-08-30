use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// 浏览器 UA 池，按请求轮换
const UAS: [&str; 5] = [
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/122.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/121.0.0.0 Safari/537.36",
    "Mozilla/5.0 (Windows NT 10.0; Win64; x64; rv:123.0) Gecko/20100101 Firefox/123.0",
    "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/17.3 Safari/605.1.15",
];

/// splitmix64 随机源：计数器 + 首次调用用系统时间播种，无第三方依赖
static SEED: AtomicU64 = AtomicU64::new(0);

fn next_rand() -> u64 {
    let mut seed = SEED.load(Ordering::Relaxed);
    if seed == 0 {
        let t = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|d| d.as_nanos() as u64)
            .unwrap_or(0x9E37_79B9_7F4A_7C15);
        seed = t | 1;
    }
    loop {
        let next = splitmix64(seed);
        match SEED.compare_exchange(seed, next, Ordering::Relaxed, Ordering::Relaxed) {
            Ok(_) => return next,
            Err(cur) => seed = cur,
        }
    }
}

fn splitmix64(mut x: u64) -> u64 {
    x = (x ^ (x >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
    x = (x ^ (x >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
    x ^ (x >> 31)
}

/// 轮换取一个浏览器 UA
pub fn user_agent() -> &'static str {
    static N: AtomicU64 = AtomicU64::new(0);
    let i = N.fetch_add(1, Ordering::Relaxed) as usize % UAS.len();
    UAS[i]
}

/// 随机延迟 [min_ms, max_ms]，模拟人工浏览节奏
pub async fn jitter_sleep(min_ms: u64, max_ms: u64) {
    let span = max_ms.saturating_sub(min_ms);
    let j = if span == 0 {
        0
    } else {
        next_rand() % (span + 1)
    };
    tokio::time::sleep(Duration::from_millis(min_ms + j)).await;
}

/// 下载：3 次重试（指数退避 + 抖动）、https→http 降级、每请求轮换 UA、
/// 可选 Referer（防盗链）、请求前随机延迟
pub async fn fetch_bytes_with(
    client: &reqwest::Client,
    url: &str,
    referer: Option<&str>,
) -> Result<Vec<u8>, String> {
    jitter_sleep(250, 700).await;
    let mut last = String::new();
    for attempt in 0..3u32 {
        for cand in [url.to_string(), http_fallback(url)] {
            let mut req = client
                .get(&cand)
                .header("User-Agent", user_agent())
                .header(
                    "Accept",
                    "text/html,application/xhtml+xml,application/xml;q=0.9,image/avif,image/webp,*/*;q=0.8",
                )
                .header("Accept-Language", "zh-CN,zh;q=0.9,en;q=0.8")
                .header("Connection", "keep-alive");
            if let Some(r) = referer {
                req = req.header("Referer", r);
            }
            match req.send().await {
                Ok(resp) => {
                    let code = resp.status().as_u16();
                    if resp.status().is_client_error() {
                        return Err(format!("HTTP {code}"));
                    }
                    if resp.status().is_server_error() {
                        last = format!("HTTP {code}");
                        continue;
                    }
                    match resp.bytes().await {
                        Ok(b) => return Ok(b.to_vec()),
                        Err(e) => {
                            last = e.to_string();
                        }
                    }
                }
                Err(e) => {
                    last = e.to_string();
                }
            }
        }
        if attempt < 2 {
            let backoff = 800u64 << attempt;
            jitter_sleep(backoff, backoff + 1500).await;
        }
    }
    Err(last)
}

fn http_fallback(url: &str) -> String {
    match url.strip_prefix("https://") {
        Some(rest) => format!("http://{rest}"),
        None => url.to_string(),
    }
}

pub async fn fetch_html(client: &reqwest::Client, url: &str) -> Result<String, String> {
    fetch_html_with(client, url, None).await
}

pub async fn fetch_html_with(
    client: &reqwest::Client,
    url: &str,
    referer: Option<&str>,
) -> Result<String, String> {
    let bytes = fetch_bytes_with(client, url, referer).await?;
    Ok(decode_text(&bytes))
}

pub fn decode_text(bytes: &[u8]) -> String {
    match std::str::from_utf8(bytes) {
        Ok(s) => s.to_string(),
        Err(_) => {
            let (cow, _, _) = encoding_rs::GB18030.decode(bytes);
            cow.into_owned()
        }
    }
}
