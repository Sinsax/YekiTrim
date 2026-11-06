# WebUI 门槛简化方案（分层版 · 2026-11）

> 状态：P0+P1（ec86dbd）、P2/P3/P4 已实现（本地待提交）；按 P0→P1→P2→P3→P4 分批。
> 范围：`webui/src`（零后端改动）。能力清单：`docs/webui-capabilities.json`；门禁：`webui/scripts/check-simplify.mjs`。
> 原则：**隐藏 ≠ 删除**；每页一个主行动；高级项默认折叠、保留可达；一级文案白话化，技术名留副标题/tooltip。

## 1. 目标

- 从"项目已创建"到"渲染任务发出"：必要点击 ≤5（不含可选 AI 与确认弹窗），3 个路由，0 次设置访问。
- 每页唯一主按钮在首屏；空态/次要区块折叠或后置。
- 不丢功能：每个能力入口登记进清单，源码用 `data-cap` 标注；门禁双向校验。

## 2. 主流程

```
首页：开始使用（仅缺项时） + 继续上次 + 添加录像
   │ 一次点击「开始处理」= pipeline stage:"process"
   ▼
识别：转写 → 找亮点 → 剪出全部片段（同一 job，阶段进度）
   ▼
挑片段：已剪卡片墙 + 未剪候选；主按钮「一键剪出全部」/「去出片」
   ▼
出片：快速出片（默认）/ 精修（P2）
   ▼
产物：成片 + 封面 + 投稿
```

### 术语

| 旧/内部 | 一级界面 | 副标题/tooltip |
|---|---|---|
| 预处理 `prepare` | 识别 | 预处理（转写 + 事件检测） |
| 切片 `slices` | 挑片段 | 切片（按事件剪出 `clip-NN`） |
| 精修 `polish` | 出片 | 精修（分段/运镜/贴纸/封面） |
| 转写 | 识别声音 | 语音转文字（ASR） |
| 事件检测 | 找亮点 | 事件检测（热度 + LLM 评分） |
| 渲染导出 | 导出成片 | 渲染导出（Dhampir 底座） |
| LLM 剪辑 | 智能剪辑 | LLM 剪辑（重新分段 + 生成事件） |
| 入库 | 采集 | 入库（录播姬 / Webhook） |

设置标签：`识别与检测 / AI 模型 / 账号 / 采集与下载 / 用量与诊断`；route key 不动。
`STEPS`/`SETTINGS_SECTIONS` 在 `utils.js`，`KIND_ZH` 在 `api/jobs.js`；别处不再抄一份。

## 3. 页面规格

### 首页

```
[首页] 把一段录像变成可发布的片段        [运行状况: N 项待处理] [刷新]
┌ 开始使用（仅缺项时） ───────────────────────────────┐
│ ✓ 运行环境   ffmpeg / ffprobe / 出片底座             │
│ ! 字幕模型   0/2 已安装                     [去下载] │
│ ○ AI 模型（可选） 未配置                    [去配置] │
│ ! 第一段录像  还没有                        [添加录像]│
└──────────────────────────────────────────────────────┘
┌ 继续上次 ────────────────────────────────────────────┐
│ stream · 下一步：识别                       [继续 →] │
└──────────────────────────────────────────────────────┘
[＋ 添加录像]  ← 有项目时默认收起，展开现有向导
最近片段（0 片段时一行引导；有片段时保留卡片墙+筛选/搜索）
[素材库 0 视频 · 1 贴纸包 · 2 字体]                [展开]
[运行状况 ✓ 一切就绪]                              [展开]
```

- `SetupChecklist.vue`（纯展示）：运行环境 / 字幕模型 / AI 模型（可选）/ 第一段录像；全部就绪收成一行。
- 来源默认：admin → 服务器文件；否则下载器就绪 → B站；否则录播姬；`sourceTouched` 让手动选择赢。三张卡全保留。
- 素材库标题行常驻、body 默认折叠（`useDisclosure('home.mediaLibrary', false)`）。
- 运行状况默认折叠；模型下载仍走现有 `#doctor-card` 的 `ModelList`。
- 无可用来源：`AppEmptyState` + admin 去设置 / 非 admin 联系管理员 + 刷新。

### 识别

```
┌ 左：视频 + 字幕 + 处理选项 ───────────┐  ┌ 右：识别流程 ──────────────────┐
│ [video]                              │  │ ① 识别声音 ② 找亮点 ③ 剪出片段 │
│ ┌ 字幕 6 条（整行）─────────────────┐ │  │ [ 开始处理 ]  ← 唯一主按钮      │
│ │ SubtitleTrack（默认展开）          │ │  │ 进度 / 当前阶段 / 中断          │
│ └───────────────────────────────────┘ │  │ ▾ 手动切片（默认展开）          │
│ ┌ ▸ 处理选项（自动剪出/ASR/素材）──┐ │  │   入点/出点/多段                │
│ └───────────────────────────────────┘ │  └─────────────────────────────────┘
└──────────────────────────────────────┘
```

- 一键处理：`projectData.pipeline(pid, {stage:'process', resume:'auto'})`；后端映射为 extract+transcribe+detect+cut，已有阶段跳过。
- 主按钮状态：无视频禁用；无事件→开始处理；有事件→去挑片段；运行中禁用+进度。
- `prepare.autoCut` 默认 true；关闭时走现有 transcribe→detect 两段链，文案「开始识别」。
- 已有事件/片段时二次确认（`redetectWarning()` + "已剪片段不会被删除"）。
- 成功态：`NextStepBar`「去挑片段」。
- LLM 未配置：常驻琥珀条 + 去配置；不阻止运行；只有 1 个整段时补提示。
- 字幕轨道头常驻、body **默认展开**（`prepare.subtitles.v2`），整行宽；手动切片在右栏流程下方、**默认展开**（`prepare.manualSlice.v2`）。
- 处理选项从右栏移到左栏字幕下方（默认收起）；只留控件与状态，ASR/主播名/素材/分步的长说明全部进 `title`。
- 404：`AppEmptyState`（项目不存在/已移除 + 回首页 + 重新导入）；其他错误保留红条。
- 首次下载提示收进主按钮 `title`："首次会下载模型（hybrid 879 MB / single 165 MB）"。

### 挑片段

```
[ 一键剪出全部（3） ]   [⋯ 更多：重新检测 / 添加片段]
[排序][筛选][已剪切·全部/精修中/已导出][未剪切][全部]
[已剪出 3 个片段 → 去出片]
[卡片墙]
```

- 有未剪事件 → 主按钮「一键剪出全部」；全剪完 → 「去出片」；空态 → 去识别 / 手动添加。
- `NextStepBar` 在 `clips.length>0 && uncutCount===0` 时出现。
- 排序/筛选/投稿/删除/精修入口不变。

### 出片（P1 轻量；P2 快速/专业）

- P1：无 `:clip` 自动选第一个未导出片段；`?list=1` 保留列表页；预取 `dhampirStatus` 并在未就绪时禁用「开始渲染」；文案 chip「出片」、`LLM 剪辑`→「智能剪辑」、`渲染导出`→「导出成片」。
- P2：快速模式右栏三步（智能剪辑 / 画面 / 导出 + 封面），隐藏 `.props-host`/时间轴/封面设计器/三页签；`yekitrim.polish.mode` 默认 `quick`；快速卡放在 `.right-col` 之外，pro 子树保留源码字符串。

### 设置（P3）

- 识别与检测：首屏只留模式/主播名/输出目录/覆盖已有文件；VAD/LLM 订正/调参进高级。
- AI 模型：`LlmQuickSetup` 四字段 + 保存并测试；静态 presets；详细表单进「高级配置」。
- 采集与下载：两个折叠卡（未配置/读失败展开，已配置折叠）。
- 用量与诊断：用量常显；日志默认不挂载（避免一进页连 SSE）。

## 4. 能力清单与门禁

- `docs/webui-capabilities.json`：surface + 约 35 条能力；字段 `id/phase/surface/label/files/anchor/advancedOnly/firstScreen`。
- `webui/scripts/check-simplify.mjs`：
  - 清单自洽、route 存在、`/models` redirect。
  - 每个 anchor 在声明文件里恰好 1 次（剥注释）。
  - `advancedOnly` 必须在 `<details>` 内。
  - 首屏预算 + floor 棘轮。
  - `webui/src` 里每个 `data-cap` 必须登记；`--mutate` 跑原语自证。
- 现有 `check-layout-invariants.mjs` 的用户口径**不放宽**；P2 快速/专业只加 `v-if`，pro 子树源码字符串不删。
- `check-polish-page-loads.mjs` 已补 `chromium`/`chromium-browser` 候选与 `--no-sandbox`；P2 需先切 pro 再断言 `.props-host`。

## 5. 文档预算

- `components/AGENTS.md` 的封面设计器细节、`views/AGENTS.md` 的右栏细节已搬到 `docs/webui-components.md` §③ / `docs/polish-right-column.md`，卡里留指针。
- 新规则只加指针级；细节写本文件与 `webui/src/AGENTS.md`。
- 每次改卡跑 `python3 scripts/check_module_map.py`。

## 6. 分批

- **P0**：文档腾位、白话标签、`useDisclosure`/`AppEmptyState`/`NextStepBar`、能力清单 + `check-simplify` 骨架、Chrome 候选。
- **P1**：首页首启/继续/来源/折叠；识别一键处理/披露/字幕折叠/404/LLM 条；挑片段主按钮/下一步/空态；出片自动选片/渲染置灰/文案；死路修复。
- **P2**：出片快速/专业 + `polish.mode` 持久化 + 页头列表 + 智能剪辑并导出。
- **P3**：设置 LLM 快配、集成折叠、日志懒挂载、标签/文案。
- **P4**：能力清单补全/变异自证、全量门禁、dist 重建。

## 7. 验证

```bash
npm --prefix webui run build
for f in webui/scripts/check-*.mjs scripts/check-preview-audio.mjs; do node "$f" || exit 1; done
node webui/scripts/check-simplify.mjs --phase=P3
python3 scripts/check_module_map.py
python3 scripts/check_module_boundaries.py
```

已知既有红（不在本方案修）：`scripts/check-dhampir-wiring.mjs`（兄弟仓 wasm 版本漂移）、`check-emoji-decode --mutate`（变异锚点命中 3 次）。

## 8. 落地结果（2026-11）

- **P0/P1**：`ec86dbd` — 首页首启/继续/来源/折叠；识别页一块卡 + `stage:"process"` 一键处理 + 两处披露 + 字幕折叠 + 404；挑片段主按钮/NextStepBar；出片自动选片/渲染置灰；术语白话化；`docs/webui-capabilities.json` + `check-simplify.mjs`。
- **P2**：`ui.polishMode`（`yekitrim.polish.mode`，默认 quick）；快速卡（智能剪辑 / 画面 / 导出 / 封面 / 任务条）；pro 子树原样保留；`?mode=pro` 首次与同页切换都生效、手动切模式清掉查询；`check-polish-page-loads.mjs` 覆盖 quick + pro。
- **P3**：`LlmQuickSetup` + 静态 presets（服务商/地址/密钥/模型/获取模型/保存并测试）；高级配置 `<details>`；config 首屏只留模式/主播名/输出目录/覆盖；集成两张卡折叠；日志懒挂载。
- **P4**：`check-simplify` 补出片模式/`?mode=pro`/`?list=1`/retired/页面探针覆盖；全量 18 条静态门禁 + 两条结构门禁 + `VT_BASE` fixture 上的快速/专业页面探针通过；`webui/dist` 重建。
