# AGENTS.md

Rust scraper (edition 2024): crawls provincial/municipal AMR food-sampling-inspection (食品抽检) result pages listed in `website.md`, downloads the month's "合格" tables into `YYYY-MM/` named `地区-第xx期.ext`, then scans them for the `search` terms in `config.toml` and writes `result.md`.

## Commands
- `cargo run` — full crawl + search (may take 10+ min: 13 regions, slow gov sites, one 134 MB PDF)
- `cargo run -- --search` — search only: rescan the existing `YYYY-MM/` folder (skips crawling; errors if folder missing)
- `cargo check` / `cargo build --release` — compile. No tests/CI. `cargo fmt` + `cargo clippy` keep it clean.
- All runs append a timestamped log to `logs/{month}.log` and print the same timestamped lines to the terminal.

## Module map (modular since Aug 2026)
- `src/main.rs` — orchestrator: config → crawl loop → search. Region-name mapping and mode dispatch live in `crawl.rs`/`config.rs`, not here. Runs on `#[tokio::main]`; regions crawl in parallel via `tokio::task::JoinSet` (one task per region, completion order arbitrary).
- `config.rs` — `REGIONS` table, `month` + `search` parsers, seed URLs, domain→名 lookup (unmapped hosts are skipped with 「无法归属地区」)
- `http.rs` — async `reqwest` (rustls, HTTP/1.1, cookie store) with 3 retries (exponential backoff + jitter), 120s timeout, https→http fallback, UTF-8→GB18030 decode, per-request UA rotation, browser Accept/Accept-Language headers, optional Referer (anti-hotlink), random pre-request jitter (built-in splitmix64, no external rand crate)
- `html.rs` — URL absolutization, `<a>` extraction, date/title/期号 parsing, `classify()` 合格/不合格/mixed
- `crawl.rs` — three site shapes: `crawl_jiangxi` (JSON API), `crawl_datepath` (湖南/福建 URL-date), `crawl_trs` (others)
- `search.rs` — sniff magic bytes (not extensions), extract per format, build `result.md`
- `docbin.rs` — legacy .doc via `cfb` crate (binary OLE); text only, no page layout
- `log.rs` — dual logging: plain-text timestamped lines appended to `logs/{month}.log`; terminal (stdout/stderr) gets the same lines plus emoji + ANSI colors by level (TTY only, piped output stays plain). Macros `info!`/`done!`/`warn!`/`error!`/`debug!` (exported at crate root; `debug!` only active with `SW_DEBUG` env var). Logs live outside the download folder so search never scans them.
- `time.rs` — civil-date math; all wall clocks (logs, 目标月份) are UTC+8 via `TZ_OFFSET_SECS` (换时区改这一处)

## Inputs (runtime config, never build artifacts)
- `config.toml` — `month = "07"` selects target month (year from system clock); `search = [...]` drives `result.md`. Not Cargo config.
- `website.md` — one seed URL per line; runs in listed order. A new region needs BOTH a `website.md` line AND a `REGIONS` entry (src/config.rs), plus a dispatch branch in `crawl.rs` if its list page differs.

## Behavior
- Skips attachments whose name contains 不合格; downloads 合格-labeled files and single combined result tables (明细表/汇总表/通告.pdf) since most regions publish one file with both rows. Support docs (本次检验项目, 说明) are skipped. `classify` rules in `html.rs` — aligned to the label text, not the file.
- Filename `地区-第N期.ext` (from title 第N期/第N号); falls back to publish date (`ZJ-20260729.xlsx`) or `期数未知`.
- Re-runs re-download into `地区-期-2.ext` (dedupe is `-N` per folder, never skip). Delete the output folder for a clean refresh.
- `search` locations: spreadsheets → `第N行` (row index), PDF → `第N页`, Word/docs → text block/paragraph (docs have no layout-driven page numbers). Mixed/extraction failures are listed as 无法解析; unparseable formats are reported honestly, not skipped silently.

## Gotchas
- Chinese GB2312/GBK pages: GB18030 fallback when not UTF-8. Same for binary-doc text.
- 湖南 https handshake hangs → http fallback (to all URLs; `http.rs`).
- 湖北 returns HTTP 412 WAF challenge → 0 files, manual/JS needed.
- 广西 publishes on a delay → fresh month may legitimately have 0 items.
- Index pages are page-1 only (no pagination crawling); months older than the visible list need manual follow-up.
- Big scanned/COM PDFs (广州-第5期.pdf, 134MB 福建 file) can exceed the per-file 120s extraction timeout — search.rs marks them 无法解析, result.md keeps the rest.
- Calamine may fail on native-WPS `.xls` (BIFF with Chinese locale) → flagged in result 无法解析.