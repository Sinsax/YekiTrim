# 第三方组件与素材

本项目（YekiTrim）以 **Apache-2.0** 发布（见 [LICENSE](LICENSE)）。下面列出它会用到、链接、
下载或引用的第三方内容及其授权，供自建与再分发时参考。

> 本文档按"事实性说明"编写，不构成法律意见。各组件条款以各自上游的许可原文为准。

---

## 一、链接进发行二进制（静态）

发行包里的 `yekitrim.exe` 是静态链接产物，以下组件因此**随二进制一起被分发**：

| 组件 | 版本 | 授权 | 说明 |
|---|---|---|---|
| ~~espeak-ng~~ | — | — | **已不再链接（2026-09-18 起）**。本项目只用 ASR、从不调用 TTS，改用自行构建的 TTS-off 预编译归档后，它被替换为 896 B 的**零符号占位 `espeak-ng.lib`**；产物门禁 `cargo xtask scan` 扫描 5 个 espeak 特有标记为 **0 命中**。 |
| sherpa-onnx / sherpa-onnx-sys | 1.13.3（本地 fork，见 `vendor/sherpa-onnx-sys/FORK-NOTES.md`） | Apache-2.0 | 语音识别（ASR）运行时。本项目用的是 **TTS-off 归档**：`espeak-ng` / `piper_phonemize` / `ucd` 均为零符号占位库（各 896 B）。 |
| onnxruntime | 随同一预编译包 | MIT | 模型推理后端。⚠️ 其预编译包内**不含许可文件**，MIT 声明需由分发方随文档补齐（即本文档）。 |
| kaldi-* / kissfft / ssentencepiece_core / kaldifst / openfst / simple-sentencepiece | 随同一预编译包 | Apache-2.0 与 BSD 系（随各自上游） | 同一静态包内 ASR 运行时所需的其余依赖。 |
| piper_phonemize / ucd | — | — | **不再编译**：TTS-off 归档里与 `espeak-ng` 同样处理为零符号占位库。 |
| Eigen | 随同一预编译包 | MPL-2.0 | 数值线性代数（经 kaldi / sherpa 间接引入）。MPL-2.0 是**文件级** copyleft：本项目未修改其任何文件，故只须保留声明。 |
| rusqlite（`bundled`） | 0.32 | MIT（内置 SQLite 为公有领域） | 账号账本/用量账本的 SQLite 驱动。 |

⚠️ **该预编译静态包内不含任何第三方许可文件**，所以上表各组件的声明义务需由**分发方**履行。
本项目已把本文档与 `LICENSE` 列为 `cargo xtask package` 的**强制**产物（缺任一个直接报错），
随包分发即完成署名。

### 可选 feature `cover-takumi`（静态封面渲染，**yekitrim-polish 默认关 / yekitrim-cli 默认开**）

该 feature **默认不编**，所以**不在默认发行产物里**（`cargo xtask scan` 扫的就是默认产物，因此看不到这一路）。
一旦用它构建（`--features yekitrim-polish/cover-takumi`），以下组件会随二进制静态分发，
声明义务与上表同样由分发方履行：

| 组件 | 版本 | 授权 | 说明 |
|---|---|---|---|
| takumi / takumi-core / takumi-html / takumi-raster | 2.14.0 / 0.25.0 / 0.3.4 / 0.5.1 | **MIT OR Apache-2.0** | 封面静态渲染（HTML+CSS → 位图），不依赖浏览器。判定与证据见 `docs/plan/takumi-cover-capability.md`。 |
| parley / harfrust / swash 等文字排版栈 | 随 takumi | MIT OR Apache-2.0 | 文本整形、换行与字形光栅化（takumi 的传递依赖）。 |
| ICU4X（`icu_*` 系列） | 随 takumi | **Unicode-3.0** | 文本分段/断行数据。⚠️ Unicode-3.0 是宽松许可（非 copyleft），但**不在 MIT/Apache 之列** —— 再分发时要一并保留其许可声明（本表即该声明）。 |
| libwebp-sys / libwebp | 0.14.4 | BSD-3-Clause | WebP 解码（takumi 的 `image-decoding` 默认 feature 拉入）。⚠️ 它用**静态 CRT**，链接期会出现 `LNK4098: 默认库"LIBCMT"与其他库的使用冲突` —— 只在这个 feature 打开时出现，默认构建**没有**这条。 |

⚠️ **若将来把 `cover-takumi` 打进发行包，必须重跑 `cargo xtask scan` 并复核本表。**

### 构建期使用，但**不随二进制分发**

> 补充（2026-10-04）：构建期还会调用 **`rc.exe`**（Windows SDK 的资源编译器）来把品牌图标
> 编进 exe（`crates/yekitrim-cli/build.rs`）。它是**微软 SDK 自带的外部命令，不是本仓的依赖包**，
> 因此不改变下面那份包清单；找不到它时构建只发一条 `cargo:warning` 并跳过图标，仍然能编过。

⚠️ **本条曾经的最大一项已随功能删除**：`headless_chrome`（MIT）以及它那个
**GPL-3.0 的构建期依赖 `auto_generate_cdp`** —— 2026-10-01 用户拍板把本仓那条
headless Chrome 渲染腿（出片兜底腿 + 封面截图通道）**整条删除**，`headless_chrome`
依赖随之从 `yekitrim-polish` 的 `Cargo.toml` 摘掉。所以：

- `headless_chrome` / `auto_generate_cdp` **已不在依赖图里**（`cargo tree -p yekitrim-cli`
  与 `-p yekitrim-polish` 都搜不到 `chrome`）；
- 那份 **GPL-3.0 的构建期组件不再随本仓构建出现**，下面那条"唯一需要法律确认的一点"
  随之作废（留在这里只是为了让审计的来龙去脉有据可查，**它已不适用于当前代码**）。

⚠️ 这一条当初是**依赖审计挖出来的**，不是手写清单发现的：它把授权正文放在 `LICENSE.txt` 里、
没有 SPDX `license` 字段，所以任何"只匹配 `license` 字段"的清单（含本文档更早的版本）都会漏掉它。
**教训留着**：新增依赖后要按同样的方式重跑审计（看 `Cargo.lock` 里每个包的
`license` **与 `license-file`**，再按 build-dep 子树单独过一遍）。

2026-10-01 复核（`cargo tree -p yekitrim-cli`，默认 feature）：

- 全树 **317 个包**（`--prefix none` 去重后；含 build 依赖）。
- 随二进制分发的**运行时闭包共 298 个包**，**没有任何 GPL / LGPL / AGPL**；逐条统计为
  `MIT OR Apache-2.0` 176、`MIT` 57、`Unicode-3.0` 18、`Apache-2.0 OR MIT` 11、
  `MIT/Apache-2.0` 11、`Apache-2.0` 9、LLVM-exception 系 5、`Zlib` 3、
  `Unlicense OR MIT` 3、BSD 系若干、`0BSD` / `BSL-1.0`（Boost）各 1。
- `r-efi` 标注为 `MIT OR Apache-2.0 OR LGPL-2.1-or-later` —— 是**三选一**，取 MIT/Apache
  即不产生 LGPL 义务。
- **19 个 build-only 包**（`comm` 全树与运行时闭包：`autocfg` / `cc` / `find-msvc-tools` /
  `glob` / `jobserver` / `phf_codegen` / `pkg-config` / `ring` / `rustc_version` / `rustls` /
  `rustls-webpki` / `semver` / `shlex` / `string_cache_codegen` / `untrusted` / `ureq` /
  `vcpkg` / `version_check` / `webpki-roots`）。其中 `ring` / `rustls` / `rustls-webpki` /
  `webpki-roots` / `untrusted` 是 TLS 栈经 `ureq` 走的构建期分支 —— 它们**同时**在运行时
  闭包里被别处用到，列在这里只是说明"构建期也拉了一次"。
- 全树搜 **`chrome` 零命中**：GPL-3.0 那一项已随功能删除，不再是需要法律确认的点。

## 二、外部命令（不随包分发，也不链接）

| 组件 | 授权 | 说明 |
|---|---|---|
| **ffmpeg** / **ffprobe** | 取决于构建选项（LGPL-2.1+，启用 GPL 组件时为 GPL） | 本项目只以 `Command::new("ffmpeg")` 调用用户自己安装的可执行文件，**不打包、不链接**。音频提取、剪切、截图、静音检测、渲染都靠它。 |

## 三、运行时下载（不随仓库/发行包分发）

| 组件 | 授权 | 说明 |
|---|---|---|
| SenseVoice ASR 模型 | 随上游（FunAudioLLM） | 首次转写时下载到 `<exe同级>/.yekitrim/models/`。 |
| Qwen3-ASR 模型 | 随上游（Qwen） | 同上，`hybrid` 模式的填词模型。 |
| 模型/VAD 文件来源 | — | `github.com/k2-fsa/sherpa-onnx` 的 `asr-models` / `hr-files` release 产物。当前**未做校验和/签名校验**。 |
| Noto Emoji | Apache-2.0（部分 CC-BY 4.0） | 封面 emoji 特效按需从 `fonts.gstatic.com` 拉取。 |
| **yt-dlp** | **Unlicense**（公有领域） | B站链接通道的下载引擎。由管理员在设置页一键触发，从 `github.com/yt-dlp/yt-dlp` 官方 release 下载 **`yt-dlp.exe`** 到 `<exe同级>/.yekitrim/bin/` —— **不随仓库、不随发行包分发**，所以不产生再分发义务（`cargo xtask scan` 也扫不到它）。内置引擎**钉死版本与 sha256**（`yekitrim_service::downloader::registry`，当前 `2026.08.19`；实测与官方 `SHA2-256SUMS` 逐字节一致）。⚠️ 目前**只有 Windows 那份带 sha256**，其它平台禁止自动下载、需管理员用 `[ingest] ytdlp_path` 指定自备副本。 |

> ⚠️ **`[ingest] downloader` 的逃生舱**：把它填成其它值时，YekiTrim 会用用户自备的「yt-dlp 兼容」
> 可执行文件（绝对路径）。那份东西**与本项目无关、也不由本项目分发**，因此不在本文档的声明范围内；
> 使用者需自行确认其授权与合规性。

## 四、前端与渲染侧引用

| 组件 | 授权 | 说明 |
|---|---|---|
| Vue 3 / Vue Router | MIT | `webui/` 运行时依赖。 |
| Vite / @vitejs/plugin-vue | MIT | 构建工具。 |
| **GSAP** | GreenSock 标准"免费使用"许可（**非开源许可**） | `crates/yekitrim-polish/templates/index.html` 与独立预览页从 jsDelivr CDN **动态加载** 3.12.5，失败时按顺序回退 unpkg / cdnjs。因为不随包分发，这里只作说明。副作用：**出片时需能访问这些 CDN 之一**。许可原文：<https://gsap.com/standard-license> |
| Lucide | ISC | `webui/src/components/Icon.vue` 内联了其图标路径数据，属再分发，**需保留版权与许可声明**。上游：<https://lucide.dev> |
| **Noto Sans SC** | **SIL OFL-1.1** | **自带字体**（2026-10-02 起替代 LXGW WenKai Lite）：字节内嵌在 `yekitrim.exe` 里（源在 `crates/yekitrim-core/assets/fonts/NotoSansSC-Regular.ttf`），首次启动自动落盘到字体库；OFL 全文随字节一并内嵌。上游：<https://github.com/notofonts/noto-cjk>（取自 Google Fonts 的**单字重 TTF**：<https://fonts.gstatic.com/s/notosanssc/v40/>） |
| **Noto Color Emoji** | **SIL OFL-1.1** | **自带字体**：字节内嵌在 `yekitrim.exe` 里（源在 `crates/yekitrim-core/assets/fonts/NotoColorEmoji-CBDT.ttf`，取自上游 `v2020-09-16-unicode13_1`），首次启动自动落盘；OFL 全文随字节一并内嵌。上游：<https://github.com/googlefonts/noto-emoji> |

## 五、素材来源（请自行核对条款后再对外使用）

| 素材 | 说明 |
|---|---|
| `crates/yekitrim-polish/assets/sfx_catalog.json` | 音效目录，由 **効果音ラボ / 小森平**（<https://taira-komori.net/>）的音效清单构建（构建脚本 `scripts/build_komori_catalog.py`）。该站声明音效**免费可商用**，但**要求不要直接转载音效 URL** —— 本仓库随源码分发的这份目录是 URL 清单，再分发前请自行评估或改为按需构建。 |
| 音效文件本体 | 运行时下载到工程的 `sfx/` 目录，**不随仓库分发**。备选下载源为 `sounddictionary.info`。 |
| **いらすとや** 插画 | `yekitrim-polish` 可按需从 <https://www.irasutoya.com/> 检索并下载插画。该站**免费额度有限且条款较严**（例如单次使用有数量限制、禁止转售素材本身），请使用前自行确认。 |
| EmoteLab 贴纸包 | 本地贴纸库（`<exe同级>/stickers/`）由用户自行导入的贴纸包构成，**不随仓库分发**。 |
| 内置字体 | **随二进制分发**（内嵌）。两支**自带字体**（**Noto Sans SC**、Noto Color Emoji，均 SIL OFL-1.1）的字节编译进 `yekitrim.exe`，首次启动自动落盘到字体库，OFL 全文一并内嵌落盘。OFL-1.1 正文明确允许 `bundled, embedded, redistributed`，故随包分发合规。见上表与 `docs/plan/font-library.md`。 |

## 六、平台名称

README / WebUI 中出现的「bilibili」「B站」「录播姬 / BililiveRecorder」等名称，仅为说明数据来源与
兼容对象的**事实性引用**，本项目与上述平台及项目**没有隶属、合作或背书关系**。
