# inspection_stats 使用说明书

省级 / 市级市场监管食品安全抽检结果采集与检索工具（Rust 命令行）。

按 `website.md` 种子地址爬取各地区当月"合格"抽检附件，存到 `YYYY-MM/`（文件名 `地区-第xx期.ext`），再用 `config.toml` 关键词扫描附件，命中写入 `result.db`（SQLite）。

## 1. 环境要求
- **Rust 工具链**（edition 2024）。
- 联网环境（爬取政府站点，13 地区，含个别上百 MB 文件，较慢）。
- 扫描件识别依赖：本地 **mineru-open-api** CLI（MinerU Open API 云端）。安装：`uv tool install mineru-open-api`，然后 `mineru-open-api auth` 配置 token（免费获取：https://mineru.net/apiManage/token）。
- 未安装 / 未配 token 时，扫描件 / 图片型 PDF 标记 `无法解析`（不崩溃）。

## 2. 编译
```bash
cargo build --release      # 生产构建
cargo build                # 调试构建
cargo check                # 仅检查
```
无单测 / CI；`cargo fmt` 与 `cargo clippy` 用于保持整洁。

## 3. 配置
`config.toml`（运行时配置，非 Cargo 配置）：
```toml
month = "08"                       # 目标月份（年份取系统时钟）
search = ["天地壹号", "天晨"]       # 检索词，驱动 result.db
# ocr_pdf_timeout = 600            # 单 PDF 整体 OCR 超时（秒，mineru-open-api 云端识别），超时标“无法解析”
# pdf_extract_timeout = 120        # PDF 文本提取超时（秒）
# sparse_threshold = 50            # 低于此字符数视为扫描件/图片型
# snippet_len = 180                # 命中上下文片段窗口（前 1/3、后 2/3）
# tz_offset = 28800                # 时区偏移秒（东八区 +8h）
# http_timeout = 120               # HTTP 请求超时（秒）
# max_retries = 3                  # HTTP 下载重试次数
# retry_backoff_base = 800         # 重试退避基准毫秒
```
- 改 `month` 换月份，`search` 换关键词；不配 `search` 仍会下载但跳过检索（提示用 `--search` 重扫）。
- 新增地区需三处同步：`website.md` 加种子 URL、`src/config.rs` 的 `REGIONS` 加域名→地区映射、若列表页结构不同则在 `src/crawl.rs` 加分支。

## 4. 命令
| 命令 | 作用 |
| --- | --- |
| `cargo run --release` | 爬取 → 下载 → 检索 → 写 `result.db` |
| `cargo run --release -- --search` | 仅重扫已存在的 `YYYY-MM/` 目录（跳过爬取） |

> 完整流程可能 10 分钟以上：13 个地区、政府站点慢、含个别上百 MB PDF。

每次运行向 `logs/{year}-{month}.log` 追加时间戳日志；TTY 下带颜色与表情，管道为纯文本。调试日志需设 `SW_DEBUG`。

## 5. 输出
- `YYYY-MM/`：下载目录，文件名 `地区-第N期.ext`（期号取标题"第N期/第N号"，缺失回退发布日期或 `期数未知`）。跳过文件名含"不合格"的附件；重复运行重下载为 `地区-期-2.ext`（同目录 `-N` 去重）。彻底刷新请删除该目录。
- `result.db`（SQLite，每次运行重建）：
  - `results(term, file, loc, snippet, folder, run_at)`：每条命中，`loc` 为 `第N页` / `第N行` / `第N段`，`snippet` 为命中上下文片段；
  - `unparsed(file, folder, run_at)`：无法解析的文件。
- `logs/{month}.log`：运行日志。

> `.gitignore` 已忽略 `result.db` / `config.toml` / `website.md` / `/target`；`YYYY-MM/` 下载目录（常达数百 MB）**不忽略**，勿 `git add .` 误提交。

## 6. 检索定位与命中
- 表格（xlsx / xls）→ `第N行`（含工作表名）；PDF → 文本型 `第N页`、OCR 型 `全文`；Word（doc / docx）→ `第N段` / 全文；其他文本 → `文本` / `全文`。
- 命中时终端逐条打印 `[命中] 文件 | 位置 | 词 | 片段`，并存入 `results` 表。混合内容或提取失败的文件标 `无法解析`，不会静默跳过。

## 7. 扫描件 / 图片型 PDF（OCR）
PDF 先经 `pdf-inspector` 判定：`Scanned` / `ImageBased` 直接走 OCR；`TextBased` / `Mixed` 先 `pdf-extract` 取文本，文本极少再回退 OCR。OCR 调用本地 `mineru-open-api` CLI（MinerU Open API 云端，`extract --ocr` 输出 markdown），整篇识别、无分页信息，命中定位为 `全文`；文件超云端限制（200MB / 600 页）或未安装 / 未配 token / 超时则标 `无法解析`。文本型 PDF 仍按 `第N页` 定位。

## 8. 已知坑
- 中文 GB2312 / GBK 页面：非 UTF-8 时回退 GB18030 解码（二进制 doc 同理）。
- 湖南 https 握手卡死 → 自动 http 回退；湖北 HTTP 412 WAF → 0 文件；广西常延迟发布 → 新月份可能 0 条。
- 列表页只抓第 1 页（不翻页）；更早月份需人工补采。
- 超大扫描 / 复合 PDF（如 134MB 福建文件）可能超 `ocr_pdf_timeout`（默认 600s，含云端上传耗时）→ 标 `无法解析`；可调大 `ocr_pdf_timeout`。
- Calamine 可能解析不了原生 WPS 的 `.xls`（带中文区域位的 BIFF）→ 标 `无法解析`。

## 9. 典型工作流
```bash
vim config.toml                 # month="08"; search=["品牌A","品牌B"]
cargo run --release             # 首次采集 + 检索
vim config.toml                 # 改 search
cargo run --release -- --search # 只重扫，不必重新下载
```
新增采集地区按 §3「三处」同步后重新编译运行。
