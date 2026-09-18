# 第三方组件与素材

本项目（V-Trim）以 **GPL-3.0-or-later** 发布（见 [LICENSE](LICENSE)）。下面列出它会用到、链接、
下载或引用的第三方内容及其授权，供自建与再分发时参考。

> 本文档按"事实性说明"编写，不构成法律意见。各组件条款以各自上游的许可原文为准。

---

## 一、链接进发行二进制（静态）

发行包里的 `vtrim.exe` 是静态链接产物，以下组件因此**随二进制一起被分发**：

| 组件 | 版本 | 授权 | 说明 |
|---|---|---|---|
| **espeak-ng** | 随 sherpa-onnx 1.13.3 预编译包 | **GPL-3.0-or-later** | 语音合成用的音素化库。**这是本项目采用 GPL-3.0 的原因**：它是 `sherpa-onnx-sys` 静态库清单里无条件 `static=` 链接的一项。 |
| sherpa-onnx / sherpa-onnx-sys | 1.13.3 | Apache-2.0 | 语音识别（ASR）运行时。其 Cargo 元数据与 `LICENSE` 只覆盖自身，**不覆盖它打包进来的 espeak-ng**。 |
| onnxruntime | 随同一预编译包 | MIT | 模型推理后端。 |
| piper_phonemize / kaldi-* / kissfft / ssentencepiece_core / ucd | 随同一预编译包 | Apache-2.0 与 BSD 系（随各自上游） | 同一静态包内的其余依赖。 |
| rusqlite（`bundled`） | 0.32 | MIT（内置 SQLite 为公有领域） | 账号账本/用量账本的 SQLite 驱动。 |

⚠️ **该预编译静态包内不含任何第三方许可文件**，所以上面 espeak-ng 的 GPL 义务需要由**分发方**自行
履行（附许可证副本 + 提供对应源码）。espeak-ng 源码：<https://github.com/espeak-ng/espeak-ng>。

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

## 四、前端与渲染侧引用

| 组件 | 授权 | 说明 |
|---|---|---|
| Vue 3 / Vue Router | MIT | `webui/` 运行时依赖。 |
| Vite / @vitejs/plugin-vue | MIT | 构建工具。 |
| **GSAP** | GreenSock 标准"免费使用"许可（**非开源许可**） | `crates/vtrim-polish/templates/index.html` 与独立预览页从 jsDelivr CDN **动态加载** 3.12.5。因为不随包分发，这里只作说明。副作用：**出片时需能访问该 CDN**。许可原文：<https://gsap.com/standard-license> |
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
