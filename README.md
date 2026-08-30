# inspection_stats 使用说明书

省级 / 市级市场监督管理局食品安全抽检（食品抽检）结果采集与检索工具。

本程序是一个 Rust 命令行工具：按 `website.md` 中的种子地址爬取各地区当月公布的"合格"抽检结果附件，下载到 `YYYY-MM/` 目录（文件名形如 `地区-第xx期.ext`），再按 `config.toml` 中的关键词扫描这些附件，把命中结果写入 `result.db`（SQLite 数据库）。

---

## 1. 环境要求

- **Rust 工具链**（edition 2024，需较新的 `cargo`）：用于编译运行。
- 联网环境：爬取的是各级政府网站，速度较慢（13 个地区，含个别大文件）。
- （可选，扫描件识别）满足其一即可：
  - **本地 Umi-OCR**（推荐）：在 Umi-OCR「全局设置」启用「开放API接口服务」（默认 `http://127.0.0.1:1224`）。无需另行安装 tesseract。
  - 或系统 `tesseract-ocr`：`poppler-utils`（`pdftoppm` 用于光栅化）+ `tesseract-ocr` + 对应语言包（如 `chi_sim.traineddata`、`eng.traineddata`）。
  两者都未安装 / 未配置时，扫描件 / 图片型 PDF 会在结果中标记为 `无法解析`，程序不会崩溃。

---

## 2. 编译

```bash
cargo build --release      # 生产构建
cargo build                # 调试构建
cargo check                # 仅检查编译
```

无单元测试 / CI。`cargo fmt` 与 `cargo clippy` 用于保持代码整洁。

---

## 3. 配置文件

### `config.toml`（运行时配置，非 Cargo 配置）

```toml
month = "08"                       # 目标月份（年份取系统时钟当前年）
ocr_lang = "chi_sim+eng"          # 可选；扫描件 OCR 使用的 tesseract 语言，缺省 chi_sim+eng
search = ["天地壹号", "天晨", "巴马世界"]   # 检索关键词，驱动 result.db
```

- 改 `month` 切换要采集的月份；改 `search` 切换要搜的品牌 / 关键词。
- 不配置 `search` 时程序仍会下载，但跳过内容检索（提示你之后用 `--search` 重扫）。

### `website.md`

每行一个地区列表页种子 URL，按行顺序采集。新增地区需同时满足三处：

1. `website.md` 增加一行种子 URL；
2. `src/config.rs` 的 `REGIONS` 表增加 `域名 → 地区名` 映射；
3. 若其列表页结构不同于现有三种，在 `src/crawl.rs` 增加对应分支。

---

## 4. 使用命令

| 命令 | 作用 |
| --- | --- |
| `cargo run` | 完整流程：爬取当月附件 → 下载 → 检索 → 写 `result.db` |
| `cargo run -- --search` | 仅检索：重扫已存在的 `YYYY-MM/` 目录（跳过爬取；目录不存在则报错退出） |
| `cargo run --release` | 同 `cargo run`，但用 release 构建（更快） |

> `cargo run` 可能耗时 10 分钟以上：13 个地区、政府站点慢、含个别上百 MB 的 PDF。

每次运行都会向 `logs/{month}.log` 追加带时间戳的日志，终端同步打印（TTY 下带颜色与表情，管道输出为纯文本）。调试日志需设置环境变量 `SW_DEBUG` 才输出。

---

## 5. 输出

- `YYYY-MM/` —— 下载目录，文件名 `地区-第N期.ext`（期号取自标题"第N期/第N号"，缺失时回退发布日期如 `ZJ-20260729.xlsx` 或 `期数未知`）。
  - 跳过文件名含"不合格"的附件；下载"合格"标签文件及合并结果表（明细表 / 汇总表 / 通告.pdf）。
  - 重复运行会重新下载为 `地区-期-2.ext`（同目录内 `-N` 去重，不会跳过）。要彻底刷新请删除该目录。
- `result.db` —— 检索结果（SQLite）。含两张表：
  - `results(term, file, loc, snippet, folder, run_at)`：每条命中，`loc` 为 `第N页` / `第N行` / `第N段` 等定位，附片段；
  - `unparsed(file, folder, run_at)`：无法解析（提取失败 / OCR 失败）的文件清单。
  - 每次运行重建该库（覆盖语义），`folder` 为对应月份目录，`run_at` 为生成时间。
- `logs/{month}.log` —— 运行日志。

> **注意**：`.gitignore` 已忽略生成输出 `result.db`、`config.toml`、`website.md`，以及 `/target`。`YYYY-MM/` 下载目录（常达数百 MB）**不会被 git 忽略**，请勿 `git add .` 误提交其中的附件。

---

## 6. 检索定位说明

- 表格（xlsx / xls）→ `第N行`（行号，含工作表名）；
- PDF → `第N页`；
- Word（doc / docx）→ `第N页`（LibreOffice 转 PDF 按页定位；soffice 缺失或转换失败时回退 `第N段` / 全文）；
- 其他文本 → `文本` / `全文`。

混合内容或提取失败的文件列为 `无法解析`；无法解析的格式会如实报告，不会静默跳过。

---

## 7. 扫描件 / 图片型 PDF（OCR）

检索 PDF 前先用 `pdf-inspector` 判定类型（`PdfType`）：`Scanned` / `ImageBased` 直接走 OCR；`TextBased` / `Mixed` 先用 `pdf-extract` 取文本，仅当文本极少时回退 OCR。OCR 流程：

1. `pdftoppm` 把每页光栅化为临时 PNG（200 DPI，存于系统临时目录，结束清理）；
2. **优先用本地 Umi-OCR 开放 API 识别**：逐页把 PNG 以 base64 POST 到 `{umi_ocr_url}/api/ocr`（`data.format=text`，中文模型 `models/config_chinese.txt`）；
3. 若 Umi-OCR 不可用（未启动 / 未开启「开放API接口服务」）或某次请求失败，自动回退 **`tesseract`** 逐页识别（整体限 600s）。

前置依赖见 §1。两种引擎任一成功即写入 `result.db`；语言包缺失或超时会将该文件标记为 `无法解析`，不影响其余文件。OCR 回退语言由 `config.toml` 的 `ocr_lang` 控制（默认 `chi_sim+eng`）；若只装了 `chi_sim`，把该项改为 `"chi_sim"` 即可。Umi-OCR 地址由 `config.toml` 的 `umi_ocr_url` 指定（默认 `http://127.0.0.1:1224`）。

---

## 8. 已知坑

- 中文 GB2312 / GBK 页面：非 UTF-8 时回退 GB18030 解码（二进制 doc 文本同理）。
- 湖南 https 握手会卡死 → 自动 http 回退。
- 湖北返回 HTTP 412 WAF 挑战 → 0 文件，需人工 / JS 处理。
- 广西通常延迟发布 → 新月份可能合法地为 0 条。
- 列表页只抓第 1 页（不做翻页）；更早月份需人工补采。
- 超大扫描 / 复合 PDF（如 134MB 福建文件）可能超过单文件 120s 提取超时或 600s OCR 超时，被标 `无法解析`，不影响其余文件。
- Calamine 可能无法解析原生 WPS 的 `.xls`（带中文区域位的 BIFF）→ 搜索自动回退 `docbin` 按 OLE 流提取文本（`全文`）；两种方式都失败才标 `无法解析`。

---

## 9. 典型工作流

```bash
# 1) 设定目标月份与关键词
vim config.toml          # month = "08"; search = ["品牌A", "品牌B"]

# 2) 首次采集 + 检索
cargo run --release

# 3) 只调整关键词，不必重新下载，直接重扫
vim config.toml          # 修改 search
cargo run --release -- --search

# 4) 换月份
vim config.toml          # month = "09"
cargo run --release
```

如需新增采集地区，按 §3 的"三处"同步修改后重新编译运行。
