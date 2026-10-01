# 第三方组件与素材

本项目（V-Trim）以 **Apache-2.0** 发布（见 [LICENSE](LICENSE)）。下面列出它会用到、链接、
下载或引用的第三方内容及其授权，供自建与再分发时参考。

> 本文档按"事实性说明"编写，不构成法律意见。各组件条款以各自上游的许可原文为准。

---

## 一、链接进发行二进制（静态）

发行包里的 `vtrim.exe` 是静态链接产物，以下组件因此**随二进制一起被分发**：

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

### 可选 feature `cover-takumi`（静态封面渲染，**默认关**）

该 feature **默认不编**，所以**不在默认发行产物里**（`cargo xtask scan` 扫的就是默认产物，因此看不到这一路）。
一旦用它构建（`--features vtrim-polish/cover-takumi`），以下组件会随二进制静态分发，
声明义务与上表同样由分发方履行：

| 组件 | 版本 | 授权 | 说明 |
|---|---|---|---|
| takumi / takumi-core / takumi-html / takumi-raster | 2.14.0 / 0.25.0 / 0.3.4 / 0.5.1 | **MIT OR Apache-2.0** | 封面静态渲染（HTML+CSS → 位图），不依赖浏览器。判定与证据见 `docs/plan/takumi-cover-capability.md`。 |
| parley / harfrust / swash 等文字排版栈 | 随 takumi | MIT OR Apache-2.0 | 文本整形、换行与字形光栅化（takumi 的传递依赖）。 |
| ICU4X（`icu_*` 系列） | 随 takumi | **Unicode-3.0** | 文本分段/断行数据。⚠️ Unicode-3.0 是宽松许可（非 copyleft），但**不在 MIT/Apache 之列** —— 再分发时要一并保留其许可声明（本表即该声明）。 |
| libwebp-sys / libwebp | 0.14.4 | BSD-3-Clause | WebP 解码（takumi 的 `image-decoding` 默认 feature 拉入）。⚠️ 它用**静态 CRT**，链接期会出现 `LNK4098: 默认库"LIBCMT"与其他库的使用冲突` —— 只在这个 feature 打开时出现，默认构建**没有**这条。 |

⚠️ **若将来把 `cover-takumi` 打进发行包，必须重跑 `cargo xtask scan` 并复核本表。**

### 构建期使用，但**不随二进制分发**

| 组件 | 版本 | 授权 | 说明 |
|---|---|---|---|
| **auto_generate_cdp** | 0.4.6 | **GPL-3.0** —— 该 crate 只带 `LICENSE.txt`，**没有 SPDX `license` 字段** | `headless_chrome 1.0.22`（MIT）的 **`[build-dependencies]`**：只在构建期生成 CDP 协议绑定代码。**不在运行时依赖闭包里**，因此**不随二进制分发**。 |

⚠️ 这一条是**依赖审计挖出来的**，不是手写清单发现的：它把授权正文放在 `LICENSE.txt` 里、
没有 SPDX `license` 字段，所以任何"只匹配 `license` 字段"的清单（含本文档此前的版本）都会漏掉它。

2026-09-18 完整审计结论（`cargo metadata --format-version 1 --locked`，全树 335 个包）：

- 随二进制分发的**运行时闭包共 312 个包**，**没有任何 GPL / LGPL / AGPL**；逐条统计为
  `MIT OR Apache-2.0` 174、`MIT` 59、`Unicode-3.0` 18、`Apache-2.0 OR MIT` 11、
  `MIT/Apache-2.0` 11、LLVM-exception 系 5、`Apache-2.0` 5、`Unlicense OR MIT` 3、
  ISC 2、BSD 系若干、`0BSD` / `Zlib` / `BSL-1.0`（Boost）各 1。
- `r-efi` 标注为 `MIT OR Apache-2.0 OR LGPL-2.1-or-later` —— 是**三选一**，取 MIT/Apache
  即不产生 LGPL 义务。
- 全图只有 **8 个 build-only 包**：`auto_generate_cdp` / `autocfg` / `bzip2` / `cc` /
  `pkg-config` / `ureq` / `vcpkg` / `version_check`。
- 唯一需要法律确认的一点：GPLv3 §2 规定"运行 GPL 程序产生的输出，只有在该输出本身构成
  covered work 时才受该许可约束"；这里生成的是声明式协议绑定数据，主流理解是**工具输出
  不算衍生作品**，且该工具本身并不随包分发。

## 二、外部命令（不随包分发，也不链接）

| 组件 | 授权 | 说明 |
|---|---|---|
| **ffmpeg** / **ffprobe** | 取决于构建选项（LGPL-2.1+，启用 GPL 组件时为 GPL） | 本项目只以 `Command::new("ffmpeg")` 调用用户自己安装的可执行文件，**不打包、不链接**。音频提取、剪切、截图、静音检测、渲染都靠它。 |

## 三、运行时下载（不随仓库/发行包分发）

| 组件 | 授权 | 说明 |
|---|---|---|
| SenseVoice ASR 模型 | 随上游（FunAudioLLM） | 首次转写时下载到 `<exe同级>/.vtrim/models/`。 |
| Qwen3-ASR 模型 | 随上游（Qwen） | 同上，`hybrid` 模式的填词模型。 |
| 模型/VAD 文件来源 | — | `github.com/k2-fsa/sherpa-onnx` 的 `asr-models` / `hr-files` release 产物。当前**未做校验和/签名校验**。 |
| Noto Emoji | Apache-2.0（部分 CC-BY 4.0） | 封面 emoji 特效按需从 `fonts.gstatic.com` 拉取。 |
| **yt-dlp** | **Unlicense**（公有领域） | B站链接通道的下载引擎。由管理员在设置页一键触发，从 `github.com/yt-dlp/yt-dlp` 官方 release 下载 **`yt-dlp.exe`** 到 `<exe同级>/.vtrim/bin/` —— **不随仓库、不随发行包分发**，所以不产生再分发义务（`cargo xtask scan` 也扫不到它）。内置引擎**钉死版本与 sha256**（`vtrim_service::downloader::registry`，当前 `2026.08.19`；实测与官方 `SHA2-256SUMS` 逐字节一致）。⚠️ 目前**只有 Windows 那份带 sha256**，其它平台禁止自动下载、需管理员用 `[ingest] ytdlp_path` 指定自备副本。 |

> ⚠️ **`[ingest] downloader` 的逃生舱**：把它填成其它值时，V-Trim 会用用户自备的「yt-dlp 兼容」
> 可执行文件（绝对路径）。那份东西**与本项目无关、也不由本项目分发**，因此不在本文档的声明范围内；
> 使用者需自行确认其授权与合规性。

## 四、前端与渲染侧引用

| 组件 | 授权 | 说明 |
|---|---|---|
| Vue 3 / Vue Router | MIT | `webui/` 运行时依赖。 |
| Vite / @vitejs/plugin-vue | MIT | 构建工具。 |
| **GSAP** | GreenSock 标准"免费使用"许可（**非开源许可**） | `crates/vtrim-polish/templates/index.html` 与独立预览页从 jsDelivr CDN **动态加载** 3.12.5，失败时按顺序回退 unpkg / cdnjs。因为不随包分发，这里只作说明。副作用：**出片时需能访问这些 CDN 之一**。许可原文：<https://gsap.com/standard-license> |
| Lucide | ISC | `webui/src/components/Icon.vue` 内联了其图标路径数据，属再分发，**需保留版权与许可声明**。上游：<https://lucide.dev> |
| LXGW WenKai（霞鹜文楷） | SIL OFL-1.1 | 封面/预览模板的字体栈**按名称引用**，不内嵌、不分发字体文件。 |

## 五、素材来源（请自行核对条款后再对外使用）

| 素材 | 说明 |
|---|---|
| `crates/vtrim-polish/assets/sfx_catalog.json` | 音效目录，由 **効果音ラボ / 小森平**（<https://taira-komori.net/>）的音效清单构建（构建脚本 `scripts/build_komori_catalog.py`）。该站声明音效**免费可商用**，但**要求不要直接转载音效 URL** —— 本仓库随源码分发的这份目录是 URL 清单，再分发前请自行评估或改为按需构建。 |
| 音效文件本体 | 运行时下载到工程的 `sfx/` 目录，**不随仓库分发**。备选下载源为 `sounddictionary.info`。 |
| **いらすとや** 插画 | `vtrim-polish` 可按需从 <https://www.irasutoya.com/> 检索并下载插画。该站**免费额度有限且条款较严**（例如单次使用有数量限制、禁止转售素材本身），请使用前自行确认。 |
| EmoteLab 贴纸包 | 本地贴纸库（`<exe同级>/stickers/`）由用户自行导入的贴纸包构成，**不随仓库分发**。 |
| 内置字体 | **已移除**，不再随包分发任何字体文件。 |

## 六、平台名称

README / WebUI 中出现的「bilibili」「B站」「录播姬 / BililiveRecorder」等名称，仅为说明数据来源与
兼容对象的**事实性引用**，本项目与上述平台及项目**没有隶属、合作或背书关系**。
