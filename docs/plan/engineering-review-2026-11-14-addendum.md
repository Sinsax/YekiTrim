# 2026-11-14 review 增补：第二轮复核（针对当前工作树）

> 对象：`docs/plan/engineering-review-2026-11-14.md`（上一轮无人值守 review）之后的工作树。
> 目的：上一轮文档第 4 节的「未修 P2」清单写成于工作树的较早时点，之后又有多处修复落地。本轮逐条回查代码，
> 把已过时的条目标清楚，并确认剩余项。**本轮没有新增代码改动。**

## 1. 本轮做了什么

| 项 | 结果 |
|---|---|
| `python3 scripts/check_module_map.py` | ✅ 35 模块 / 7 项检查 |
| `python3 scripts/check_module_boundaries.py` | ✅ 19 条 / 656 文件 / 0 违规 |
| `npm --prefix webui run build` | ✅ 成功。源文件产物逐字节不变，只有 `webui/dist/.built-from` 的 src 指纹从 `5a729636bac896b1` 刷新为 `2fa52eeec2d39e8f`（旧指纹与当前 src 不符） |
| `webui/scripts/check-*.mjs` + `scripts/check-preview-audio.mjs` | ✅ 全部 exit 0（`check-polish-page-loads` 与 `check-overlay-pixels` 按设计 SKIP / 需要参数） |
| Rust 侧 `cargo check` / `cargo test` | ⚠️ **本机没有安装任何 Rust toolchain**（`rustup` 无默认版本），本轮无法编译或运行。Rust 侧结论全部来自读代码，**未经编译验证**。上一轮报告里的「cargo test 全绿」在本轮无法复现，需要在有工具链的机器上重跑 |

## 2. 上一轮「未修 P2」清单的当前状态

「已修」指当前工作树里代码已有对应修复（注释里带 review 编号的，已逐处核对）。

| 原编号 | 问题 | 当前状态 |
|---|---|---|
| B1 | SSE 终态事件被 `take_while` 吃掉 | ✅ 已修（`api/pipeline.rs`，改用 `unfold`，先发终态再关流） |
| B2 | `/api/recorder/{objects,recordings}` 缺归属过滤 | ✅ 已修（`api/rooms.rs` 与 `owned_room` 同口径，带回归测试） |
| B3 | `PUT /api/config/global` 非原子写 | ✅ 已修（临时文件 → 真反序列化 → rename） |
| B5 | 用户层 `fonts/import` 可读服务器任意路径 | ✅ 已修（`font_import_allowed` 限定到用户自己的根，附两用户测试） |
| B6 | `/api/health` 公开回 `fonts_dir` 绝对路径 | ✅ 已修（只回布尔与计数） |
| B4 / B7 / B8 | 项目 `missing` 恒 false；产物增长无配额；LLM `base_url` 出网 | ❌ 未修，见第 3 节 |
| P1 | `[asr] model` 只影响下载清单，运行时固定 SenseVoice | ❌ 上轮结论，本轮未重新核实，见第 3 节 |
| P2 | `publish_config.json` 写入不存在的 `clip.dm.srt` | ✅ 已修（写入前判存在） |
| P5 | 部分剪切失败仍算成功 | ❌ 未修，属产品取舍（D9） |
| P6 / P7 / P9 | 共享 recognizer、锚点覆盖率常量、空目录入索引 | ❌ 上轮结论，本轮未复核 |
| P8 | 多段 `duration` 用跨度 | ✅ 已修（`clip.json` 与发布清单用内容时长） |
| E1 | 无音轨素材整条出片失败 | ✅ 已修（无音轨段补静音，不回落到主素材） |
| E2 | ffmpeg 调用无超时 | ✅ 渲染路径基本已走 `run_with_timeout`（23 处）。剩余裸 `.output()` 仅 `ffmpeg.rs:2227`（测试）、`dhampir.rs:1003`（`node --version`）、`dhampir.rs:1121`（底座 CLI 调用，未设超时，未修） |
| E3 | 失败路径残留 `_temp_*` | ✅ 代码已有清理分支（`render/pipeline.rs` 注释标明），本轮未逐路径实测 |
| E5 | `cover.image` 覆盖关键帧 | ✅ 已修（`_cover_frame.jpg` 只在不存在时播种） |
| F2 | B 站下载进度轮询可能永不停止 | ✅ 已修（连续 8 次查不到即放弃；25 分钟兜底上限） |
| F3 | 设置页切子页丢未保存表单 | ✅ 已修（`SettingsView.vue` 有 `onBeforeRouteLeave` 与 `formDirty`） |
| D1 | `[cutting] overwrite` 死字段 | ✅ 已接通（`cutting/mod.rs::skip_reason`，`overwrite=false` 时跳过自家已成片） |
| — | 上轮 P1-1～P1-12 | ✅ 本轮逐条核对，代码均在（jobs 终态守卫、channel-map 409、titles 默认值、版本键、预览走 `load_project`、写闸、控制行换行等） |

## 3. 仍然开放、需要用户拍板或后续处理

这些都**不属于「明显问题」**，本轮不动：

1. **ASR 模型配置是否真的生效**（上轮 P1 / D10）：`[asr] model="qwen3-asr"` 是否只影响下载清单，需要在有工具链的机器上确认后再决定「真实现」还是「删字段」。
2. **部分剪切失败的语义**（P5 / D9）：1 条 ffmpeg 失败时任务算成功还是失败。
3. **出片/转写产物的配额**（B7 / D14）：当前只拦入库与下载。
4. **LLM `base_url` 的出网限制**（B8 / D3）：默认单人保留，还是对多用户默认收紧。
5. **项目 `missing` 判据**（B4）：`root_dir.is_dir()` 是否纳入列表判断。
6. **多用户入口**（D2）：设置/素材库对普通账号开放与否（后端已按 Owner 分）。
7. **前端性能**（U1 / U2 / D6）：精修页单次冷加载约 42 个 `/api` 请求，首页首屏 `fs/list` 约 19 次，叠加全局限流会自撞 429。上轮未修，需要决定收单飞的方案。
8. **UI 细节包**（D7，本轮未复核）：「全部」双按钮、卡片角标裁切、封面 1280 属性列、首页卡片墙宽度。
9. **术语统一**（F4 / D12）：「入库 / 切片 / 精修」与新的「采集 / 挑片段 / 出片」是否统一。
10. **画质档位**（E7）：没有质量/分辨率/码率配置项，是否需要。
11. **预览端口表**（E6）：按片段常驻、无淘汰。

## 4. 建议

- 在有 Rust 工具链的机器上先跑一次：`cargo check --workspace --all-targets` 与 `cargo test --workspace --no-fail-fast`（真库路径见上一轮报告第 3 节），确认 Rust 侧仍然全绿。
- 以上第 3 节的 1～6 项需要产品口径，建议你先拍板，再开修复。

## 5. 复核的局限

- 本轮没有启动服务、没有跑截图和端到端，只做了读代码 + 前端构建与静态门禁。
- Rust 侧的「已修」判断基于代码与注释核对，未经编译和测试。
