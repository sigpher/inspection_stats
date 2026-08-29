# AGENTS.md

Rust scraper (edition 2024): crawls provincial/municipal AMR food-sampling-inspection (食品抽检) result pages listed in `website.md`, downloads the month's "合格" tables into `YYYY-MM/` named `地区-第xx期.ext`, then scans them for the `search` terms in `config.toml` and writes `result.db` (SQLite).

## Commands
- `cargo run` — full crawl + search (may take 10+ min: 13 regions, slow gov sites, one 134 MB PDF)
- `cargo run -- --search` — search only: rescan the existing `YYYY-MM/` folder (skips crawling; exits 1 if folder missing)
- `cargo check` / `cargo build --release` — compile. No tests/CI. `cargo fmt` + `cargo clippy` keep it clean.
- Every run appends a timestamped log to `logs/{year}-{month}.log` (plain text; the terminal gets the same lines with colors).
- `config.toml` + `website.md` are read at runtime → changing month/search/OCR settings needs NO rebuild. `REGIONS` and `crawl.rs` are compiled code → adding a region requires `cargo build`.

## Module map (modular since Aug 2026)
- `src/main.rs` — orchestrator: config → crawl loop → search. The `--search` mode dispatch lives here; region-name mapping lives in `config.rs`. Runs on `#[tokio::main]`; regions crawl in parallel via `tokio::task::JoinSet` (one task per region, completion order arbitrary).
- `config.rs` — `REGIONS` table (13 regions), domain→名 lookup (unmapped hosts are skipped with 「无法归属地区」), and regex parsers for every `config.toml` key. No toml/serde crate — keys are matched loosely, so preserve the documented shapes; a new key is NOT read unless you add a parser here.
- `http.rs` — async `reqwest` (rustls, HTTP/1.1, cookie store) with 3 retries (exponential backoff + jitter), 120s timeout, https→http fallback, UTF-8→GB18030 decode, per-request UA rotation, browser Accept/Accept-Language headers, optional Referer (anti-hotlink), random pre-request jitter (built-in splitmix64, no external rand crate)
- `html.rs` — URL absolutization, `<a>` extraction, date/title/期号 parsing, `classify()` 合格/不合格/mixed
- `crawl.rs` — three site shapes: `crawl_jiangxi` (JSON API), `crawl_datepath` (湖南/福建 URL-date), `crawl_trs` (others)
- `search.rs` — sniff magic bytes (not extensions), extract per format, build `result.db`. PDF 先用 `pdf-inspector` 判定 `PdfType`（TextBased/Scanned/ImageBased/Mixed）：Scanned/ImageBased 直接走 OCR；TextBased/Mixed 先 `pdf-extract` 取文本，文本极少再回退 OCR。OCR 结果无分页信息 → `loc` 为「全文」；文本型 PDF 仍按「第N页」。
- `ocr.rs` — 扫描件/图片型 PDF 识别：调用本地 **mineru-open-api** CLI（MinerU Open API 云端，`extract --ocr <pdf>` 输出 markdown 到 stdout），`ocr_pdf_timeout` 作整体超时。依赖已安装 CLI + token（`mineru-open-api auth`）；未安装/未配 token/超限 → 该文件标记「无法解析」，无 tesseract / Umi-OCR 回退。仅在 pdf-extract 失败或文本极少时触发。
- `docbin.rs` — legacy .doc via `cfb` crate (binary OLE); text only, no page layout
- `log.rs` — dual logging: plain-text timestamped lines appended to `logs/{year}-{month}.log`; terminal (stdout/stderr) gets the same lines plus emoji + ANSI colors by level (TTY only, piped output stays plain). Macros `info!`/`done!`/`warn!`/`error!`/`debug!` (exported at crate root; `debug!` only active with `SW_DEBUG` env var). Logs live outside the download folder so search never scans them.
- `time.rs` — civil-date math; all wall clocks (logs, 目标月份) are UTC+8 via `config.toml` `tz_offset` (default 28800) — change timezone there, not in code.

## Inputs (runtime config, never build artifacts)
- `config.toml` — `month = "08"` selects target month (year from system clock); `search = [...]` drives `result.db`. OCR tunables (all optional, defaults in parentheses): `ocr_pdf_timeout`(600) 单 PDF 整体 OCR 超时（mineru-open-api 云端识别）, `pdf_extract_timeout`(120) pdf-extract 超时, `sparse_threshold`(50) 判定扫描件的字符阈值. Other tunables: `snippet_len`(180) 命中上下文窗口, `tz_offset`(28800) 时区偏移秒, `http_timeout`(120) HTTP 请求超时, `max_retries`(3) 下载重试, `retry_backoff_base`(800) 重试退避基准毫秒. Not Cargo config.
- `website.md` — one seed URL per line; runs in listed order. Adding a region needs BOTH a `website.md` line AND a `REGIONS` entry (src/config.rs), plus a `crawl.rs` dispatch branch if its list page differs — then rebuild.

## Behavior
- Skips attachments whose name contains 不合格; downloads 合格-labeled files and single combined result tables (明细表/汇总表/通告.pdf) since most regions publish one file with both rows. `classify` rules in `html.rs` run on the label text (not the file): any label containing a data marker (附件/明细/汇总/信息表/结果/通告/公告/抽检/监测/检测/台账/报告/数据/表) is kept — so support docs labeled `附件N：检验项目` / `附件N：说明` ARE downloaded; only labels without 合格/不合格 and without any marker (e.g. bare 说明) are skipped.
- Filename `地区-第N期.ext` (from title 第N期/第N号); falls back to publish date (`ZJ-20260729.xlsx`) or `期数未知`.
- Re-runs re-download into `地区-期-2.ext` (dedupe is `-N` per folder, never skip). Delete the output folder for a clean refresh.
- `result.db` is deleted and recreated on every search run (both the end of `cargo run` and `--search`). Schema: `results(term, file, loc, snippet, folder, run_at)` + `unparsed(file, folder, run_at)`.
- `search` locations: spreadsheets → `第N行` (含工作表名), PDF → `第N页`, docx → `第N段`, binary .doc → `全文`, other text → `文本`. Mixed/extraction failures are listed as 无法解析; unparseable formats are reported honestly, not skipped silently.
- Snippet matching strips ALL whitespace from both text and term (handles OCR-inserted CJK spacing like `天 地 壹 号`) and is case-insensitive.

## Git hygiene
- `.gitignore` excludes `/target`, `result.db`, and all download extensions (`*.pdf/*.xlsx/*.doc/*.png/...`), so the `YYYY-MM/` 下载目录 contents stay untracked — `git add .` won't add them.
- BUT `config.toml`, `website.md`, `result.md`, `logs/*.log`, and `inspection_stats.zip` are ALREADY TRACKED (committed before the ignore rules / force-added). `.gitignore` does not protect tracked files, so `git add .` WILL stage your edits to them. `result.md` and `inspection_stats.zip` are stale leftovers from an older version — current code writes only `result.db`.

## Gotchas
- Chinese GB2312/GBK pages: GB18030 fallback when not UTF-8. Same for binary-doc text.
- 湖南 https handshake hangs → http fallback (to all URLs; `http.rs`).
- 湖北 returns HTTP 412 WAF challenge → 0 files, manual/JS needed.
- 广西 publishes on a delay → fresh month may legitimately have 0 items.
- Index pages are page-1 only (no pagination crawling); months older than the visible list need manual follow-up. Exception: 江西 (`crawl_jiangxi`) paginates its JSON API itself (up to 10 pages).
- Big scanned/COM PDFs (广州-第5期.pdf, 134MB 福建 file) can exceed the per-file extraction/OCR timeouts — search.rs marks them 无法解析, result.db keeps the rest.
- Calamine may fail on native-WPS `.xls` (BIFF with Chinese locale) → flagged in result 无法解析.
- 扫描件/图片型 PDF：先用 `pdf-inspector` 判定类型，Scanned/ImageBased 直接走 OCR；其余先 `pdf-extract` 取文本，文本极少再回退 OCR（`ocr.rs`）。**用本地 `mineru-open-api` CLI（MinerU Open API 云端）**：需先 `uv tool install mineru-open-api` + `mineru-open-api auth` 配 token；未安装/未配 token/文件超云端限制（200MB/600 页）→ 该文件列为 无法解析，不会崩溃（无 tesseract / Umi-OCR 回退）。
- OCR 单个 PDF 整体限 `ocr_pdf_timeout`(600s)，超时按 无法解析 处理；超大扫描件（如 134MB 福建 PDF）或大文件上传云端耗时可能仍超时被跳过。MinerU 输出无分页信息 → 命中 loc 为「全文」。