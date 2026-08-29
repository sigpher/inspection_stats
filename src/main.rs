mod config;
mod crawl;
mod docbin;
mod html;
mod http;
mod log;
mod ocr;
mod search;
mod time;

use std::fs;
use std::path::Path;
use std::time::Duration;

const UA: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0 Safari/537.36";

#[tokio::main]
async fn main() {
    let month = config::month_from_config();
    let (year, _, _) = time::civil_from_days(time::sys_now());
    let folder = format!("{year}-{month:02}");
    let dir = Path::new(&folder);
    log::init(&folder);

    let terms = config::search_terms();
    if std::env::args().any(|a| a == "--search") {
        if !dir.exists() {
            error!("{folder} 目录不存在，请先运行 cargo run 下载");
            std::process::exit(1);
        }
        search::run(&terms, dir);
        return;
    }

    fs::create_dir_all(dir).expect("无法创建下载目录");

    let mut headers = reqwest::header::HeaderMap::new();
    headers.insert(
        reqwest::header::ACCEPT,
        "text/html,application/xhtml+xml,application/xml;q=0.9,*/*;q=0.8"
            .parse()
            .unwrap(),
    );
    headers.insert(
        reqwest::header::ACCEPT_LANGUAGE,
        "zh-CN,zh;q=0.9,en;q=0.8".parse().unwrap(),
    );

    let client = reqwest::Client::builder()
        .user_agent(UA)
        .timeout(Duration::from_secs(120))
        .cookie_store(true)
        .default_headers(headers)
        .build()
        .expect("无法创建 HTTP 客户端");

    let sites = config::read_sites();
    let mut tasks = tokio::task::JoinSet::new();
    for seed in &sites {
        let Some(name) = config::region_of(seed) else {
            error!("[跳过] 无法归属地区: {seed}");
            continue;
        };
        let client = client.clone();
        let dir = dir.to_path_buf();
        let seed = seed.clone();
        tasks.spawn(async move {
            let got = crawl::run_region(&client, &seed, name, month, year, &dir).await;
            (name, got)
        });
    }

    let mut total = 0usize;
    while let Some(res) = tasks.join_next().await {
        match res {
            Ok((name, got)) => {
                total += got;
                done!("[完成] {name}: 共下载 {got} 个文件");
            }
            Err(e) => error!("[并行任务失败] {e}"),
        }
    }
    done!("\n全部完成，共下载 {total} 个文件，保存在 ./{folder} 目录。");

    if terms.is_empty() {
        info!(
            "config.toml 未配置 search 词，跳过文件内容搜索（在 config.toml 中配置 search 后执行 cargo run -- --search）"
        );
    } else {
        search::run(&terms, dir);
    }
}
