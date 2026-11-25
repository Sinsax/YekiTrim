# T14 第 3 步（删模板腿）影响面 — 2026-11-25

> 状态：**未执行**。这是 B1（现在就删）的施工图；B2 是「等下个版本」。
> 依据：`docs/plan/remediation-plan-2026-11-24.md` §P3 T14 第 3 步。
> 第 1、2 步与「前半」（删两条死路由 + PreviewPorts）已完成，见执行记录。

## 1. 现在还剩什么（也就是要删什么）

| 对象 | 规模 | 现在谁在用 | 删后怎么办 |
|---|---|---|---|
| `crates/yekitrim-polish/templates/index.html` | 3,229 行 / 150 KB | ①`render/server.rs` 的 `include_str!`（预览 HTML）；②`project/config.rs` 一条契约测试；③**12 个门禁脚本**当参照 | 全删；参照职责交给 `engine.js` + `timeline.rs` |
| `render/server.rs::INDEX_HTML` / `build_html` / `RenderServer::new·start·start_with` | 约 400 行（文件 960 行里的模板/监听那部分） | 进程内预览：`commands::cmd_preview`（`polish test` 的最后一步） | 删；`polish test` 止于 process，不再起预览 |
| `render/server.rs` 的**文件服务**部分（`start_file_server` / `font_face_css*` / `fallback_stack` / `preferred_fallback_family` / `build_sticker_map`） | 约 560 行 | `start_file_server` 只有 `cmd_preview` 与（已删的）HTTP 预览用；**字体那族被出片/封面用着** | 只删文件服务与监听壳，**字体相关全留** |
| `crates/yekitrim-polish/src/commands.rs::cmd_preview` | 约 120 行 | `cmd_test`（`polish test`） | 删；`cmd_test` 最后一步改成「process 完成即结束」 |
| `project/config.rs::template_stage_hooks_match_contract` | 约 40 行（`include_str!` 在 4267） | 自己 | 改成断言 `engine.js` + `timeline.rs` 两个消费面 |
| `crates/yekitrim-polish/templates/AGENTS.md` | 10 KB | 该目录的卡 | 随目录一起删（`templates/cover.html` 仍在，卡要保留并重写；`module-map.json` 的 `polish-templates` 条目指向它） |
| `portrait_backdrop.rs` / `tests/common/mod.rs` 等注释里的「参照实现」指路 | 几行注释 | 人 | 改指 `engine.js` / 转译器 |

## 2. 真正的工作量在门禁脚本（12 个）

这些脚本把 `templates/index.html` 当**参照实现**的一侧，删掉后每一条都要重新划判据边界：

```
scripts/check-camera-zoom-parity.mjs      2 处   （档位倍率五处对拍里的「模板」一列）
scripts/check-effect-registry.mjs         1 处   （每效果的模板锚点 anchors.*）
scripts/check-font-inputs.mjs             3 处
scripts/check-noise-parity.mjs            2 处
scripts/check-noise-real.mjs              1 处
scripts/check-preview-engine.mjs          1 处   （桶清单必须与模板同集合）
scripts/check-yekitrim-translator.mjs     3 处   （几何抽 stageDrawRect 闭包与模板逐值对拍）
webui/scripts/check-caption-wrap.mjs      2 处   （折行两份实现）
webui/scripts/check-color-parity.mjs      2 处
webui/scripts/check-emoji-decode.mjs      1 处
webui/scripts/check-handheld.mjs          1 处   （手持摇摆两个 JS 副本之一）
webui/scripts/check-layout-invariants.mjs 1 处
webui/scripts/check-overlay.mjs           3 处   （覆盖层色档四处实现之一）
webui/scripts/check-sticker-anchor.mjs    1 处
```

处置原则（按 D-1 口径）：**参照侧只剩「转译器 + timeline 数学」**，DOM 腿降级为近似。
所以每一处「模板 ↔ engine.js」的两侧对拍，改成：
- **保留数值判据**（时刻 / 落点 / 描边宽度 / 倍率 / 色档规则）：`timeline.rs`（或转译器）↔ `engine.js`；
- **删掉「模板独有」的那一列**（例如 `stageDrawRect` 的模板闭包抽取、桶清单与模板同集合）；
- 每条都要附**变异自证**，证明改完还在判断东西（这批脚本原本就有 `--mutate`/`--self-test` 基建）。

另外 `scripts/gate.mjs` 的映射表里有一条把 `templates/**` 映到上述脚本 —— 模板没了要一起改。

## 3. 建议的执行顺序（B1 时照这个走，每步都能独立验证）

1. **先改门禁**（12 个脚本 + gate 映射），此时模板还在、可以作为「改判据前先跑一遍拿到基线」的对照；
2. 再删 `project/config.rs` 的模板契约测试，改成 `engine.js` + `timeline.rs` 两侧；
3. 再删 `cmd_preview`（`polish test` 止于 process）与 `render/server.rs` 的监听/模板壳（字体那族保留）；
4. 最后删 `templates/index.html` + 重写 `templates/AGENTS.md`（`cover.html` 保留）；
5. 收尾：`cargo test --workspace`、`node scripts/gate.mjs --all`（期望 0 条非基线失败）、
   `npm --prefix webui run build`（前端不涉及，但确认无引用）、`route_manifest` 前后一致（142→140 之后应保持 140）。

## 4. 风险与为什么要「跨一个版本」

- **参照实现消失**：`templates/index.html` 是唯一一份「曾经逐帧渲染、且被 12 条门禁当参照」的实现。
  删掉后，若某个 parity 判据写得不够严，就再没有第二个视角能发现漂移 —— 这正是计划要求「第 1、2 步稳定一个版本后再删」的原因。
- **`polish test` 的能力下降**：它现在会起一次预览让人肉眼看，删后只到 `process`。
  （可用 `yekitrim polish preview <clip>`（有 `yekitrim.toml` 时起真服务）替代。）
- **不可逆**：模板在 git 历史里，但 3,229 行的 canvas 实现与 12 条判据的对应关系一旦断开，重建成本很高。

## 5. 结论

- **B1（现在做）**：约 1 个工作日量级，改动 15 个文件（12 脚本 + `server.rs` + `config.rs` + `commands.rs` + `index.html` + 卡），
  一次提交或两次（先门禁、后删码）。
- **B2（等下个版本）**：保持现状 —— 模板只剩 `polish test` 与 12 条门禁当参照，不影响任何用户可见行为。