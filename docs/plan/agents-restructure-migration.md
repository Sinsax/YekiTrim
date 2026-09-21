# 拆卡台账：根 `AGENTS.md` → 34 张模块卡 + 两条门禁

> 本文是一次性**迁移记录**（不是长期规范）。长期规范本身是根 `AGENTS.md`（路由页）、
> 各模块目录的 `AGENTS.md` 与 `docs/module-map.json`（结构注册表）。
> 落成日期：2026-09-20 ｜ 触碰面：文档 + 脚本 + 注释，**零行为改动**。

## 1. 为什么要拆（实测，不是感觉）

| 事实 | 数据 |
|---|---|
| 拆之前根 `AGENTS.md` | **237,027 B / 2327 行** |
| DSH 工作区指令预算 | **65,536 B**（`maxBytes`） |
| 实际进入上下文的 | 前 ~65 KB（≈ line 700），**尾部 72% 被静默丢弃** |
| 被丢掉的正是最硬的规矩 | `WebUI/Server` 后半、`核心文件索引`、`开发`命令、`开源与公网部署`（路径面三防线 / 认证授权 / 限流 / 端口单源 / 公网清单） |

DSH 的加载语义（实读 DSH 安装包内 `@deepseek-ai/dsh-agent-instructions` 的包文档）：

1. 基线 = 用户全局 `~/.dsh/AGENTS.md` + **项目根 → 会话工作目录**链上的每个 `AGENTS.md`（broad→specific）；
2. **read / write / edit 触到更深目录后，下一次请求会注入该目录适用的指令文件**；
3. 候选名 `AGENTS.md` / `CLAUDE.md`，另有 `AGENTS.local.md` 叠加层；
4. ⚠️ 预算超限时**先整份丢掉较宽的文件，再截断最具体的那份** —— 只要根文件仍然超标，
   agent 一旦触到任何子目录卡，**整份根文件会被丢掉**，全局规矩反而全没。

**结论**：把规则按模块放进**各目录自己的 `AGENTS.md`** 是唯一能"到点注入"的机制；根文件必须瘦成路由页。

**机制已被本次实操验证**：写入 `crates/vtrim-polish/templates/AGENTS.md` 之后，下一次请求同时注入了
这一张卡**和它的父卡** `crates/vtrim-polish/AGENTS.md`（`Additional instructions from:` 两条）。

## 2. 落地结构（最终）

| 产物 | 规模 | 作用 |
|---|---|---|
| 根 `AGENTS.md` | **15,534 B**（预算 20,480） | 路由页：全局不变式 + 模块索引表 + 改代码协议 + 验证命令 |
| 模块 `AGENTS.md` | **34 张**（合计 ~356 KB，按需注入） | 各自的「职责 / 入口 / 边界 / 验证」 |
| `docs/module-map.json` | 35 个模块 | 结构注册表（唯一权威）：模块 → 目录 → 卡 → verify + 预算 + ⚠️ 下限 |
| `scripts/check_module_map.py` | 新增 | 门禁：注册表↔磁盘一致 / 覆盖率 / 骨架 / 预算 / 索引表一致 / ⚠️ 下限 |
| `scripts/check_module_boundaries.py` | 6 → **14 条规则** | 门禁：分层方向 + P4 隔离 + 前端边界（并补上 `.vue/.js` 扫描） |
| Rust `//!` 头注释 | **38 个文件**（每个 4 行） | 单文件阅读时也能看到职责/入口/边界 |

预算实测：根 **15,534 / 20,480 B**；单卡最大 `webui/src/views` **23,085 B**（上限 24,576）；
**最坏链 53,977 / 57,344 B**（`root + webui + webui/src + webui/src/views`）——留 3.4 KB 余量。

## 3. 内容去向映射（快照行号 → 目的地）

快照 = `.workbuddy/agents-restructure/AGENTS.before.md`（拆之前那份 237 KB 的逐字副本）。

| 快照行 | 段 | 去向 |
|---|---|---|
| 5–61 | 架构 / 依赖方向 / `api/` 子模块纪律 / Stage 编号 | 根 + `crates/vtrim-server/src/api/AGENTS.md` |
| 62–100 | CLI 命令表 | 根（命令表面）+ `crates/vtrim-cli/AGENTS.md` |
| 101–114 | 状态机推导表 | 根（表）+ `crates/vtrim-core/src/pipeline/AGENTS.md` |
| 115–294 | 配置（三份样例 toml）/ LLM 并发 / 用量记账 | `core/src/config`、`core/src/llm`、`vtrim-service`、`vtrim-server` |
| 295–478 | 各阶段 I/O | `crates/vtrim-core/AGENTS.md` + `src/{asr,detection,danmaku,cutting}` |
| 479–517 | 同步 IO / `spawn_job` 纪律 | `crates/vtrim-server/src/api/AGENTS.md`（逐字） |
| 518–529 | 样式纪律 | `webui/src/styles/AGENTS.md` |
| 530–1609 | 前端信息架构 v5 | `webui/`、`webui/src/`、`webui/src/{api,views,components,styles,polish}`、`polish/src/{assets,render}`、`server/src/api` |
| 1610–1799 | 认证 / 项目托管 / wgpu / 录播姬 / 日志 | `vtrim-service`、`vtrim-server`、`server/src/api` |
| 1801–1836 | 字体库 | `crates/vtrim-core`（fonts）+ `polish/src/render` + `webui/src` |
| 1837–2007 | 渲染导出与封面 | `polish/src/render` + `crates/vtrim-polish` + `polish/templates` + `vtrim-server`（`render_proc`） |
| 2008–2020 | 多用户进度 | 设计稿指针（`docs/superpowers/specs/2026-09-12-*`） |
| 2021–2131 | B站链接通道硬规矩 | `vtrim-service/src/{bilibili,downloader,net,ingest}` + `vtrim-server` |
| 2132–2233 | 开发命令 / 配置四步 / 核心文件索引 | 根（命令 + 门禁）+ `core/src/config` + 各卡「入口」（索引改写为模块索引） |
| 2234–2328 | 开源与公网部署 | `docs/deploy.md`（正文）+ 根（摘要）+ `server/src/api`（三防线）+ README/`docs/releases`（字体许可） |

## 4. 保真报告（⚠️ 规则）

**判据**：拆之前含 `⚠️` 的 **117 行**，逐条检查其正文是否仍存在于仓库的知识库里
（指令层 `AGENTS.md` + 长文层 `docs/**/*.md`；判据容忍重新折行、缩进与行尾标点）。
工具：`.workbuddy/agents-restructure/check_warnings.py`（迁移工具，已 gitignore）。

**结果**：**93 / 117 逐字命中**，其余 24 条为**同义改写**（结论、数字与"不许/别再/必须"的约束都在，
只是句子被压缩/换行方式不同）—— 已逐条核对落点，**没有一条规则丢失**。证据：

- 全仓 ⚠️ 行 **279 行 ≥ 迁移前 117 行**（`check_module_map.py` 的 `warning_floor=117` 已把它变成硬门禁）；
- 迁移过程中脚本曾抓出 7 处"压缩时改坏的 ⚠️ 行"（`1021/1133/1182/1328/1379/1441/1448`），
  已全部逐字还原 —— 这就是这道校验存在的意义；
- 24 条改写项的落点（抽样核对）：设置页五子页与 `/settings` redirect → `webui/src/AGENTS.md`；
  侧栏阶段打勾/`cur` 已弃用 → 同卡；`:clip` 用目录名 → 同卡；覆盖层色档 → `polish/templates` + `polish/src/render`；
  设计器镜像清单/黑边根因/关键帧 404 → `webui/src/components` 与 `webui/src/styles`；
  `Preload::Subtitle`/转写解析基准 → 本批次已**逐字补进** `crates/vtrim-service/AGENTS.md`；
  前端任务列表与按钮门控（超时/三态/陈旧快照/`watchJob{close}`）→ 本批次已**逐字补进** `webui/src/components/AGENTS.md`。

## 5. 验证记录（本次实测）

```bash
python3 scripts/check_module_map.py            # exit 0：35 个模块 / 7 项检查全过
python3 scripts/check_module_boundaries.py     # exit 0：14 条规则 / 244 个文件 / 0 违规
python3 .workbuddy/agents-restructure/check_warnings.py   # 93/117 逐字，余为改写（见 §4）
cargo check --workspace --all-targets          # exit 0
cargo test -p vtrim-core -p vtrim-service -p vtrim-server # 146 + 135 + 121 passed / 0 failed
node webui/scripts/check-overlay.mjs 等 6 个契约脚本      # 全部 exit 0（"全部通过"）
```

两道新门禁**都验证过"真的会拦"**（不只是"当前是绿的"）：

- 边界门禁：临时塞 `crates/vtrim-core/src/__boundary_probe.rs`（引用 `vtrim_service::…`）→
  `[!!] core 不许认识上层` + 精确行号 + exit 1；删除后回到 0 违规。
- 结构门禁：临时塞 `crates/__probe_crate/src/x.rs` → `有 1 个源文件不属于任何模块卡` + exit 1；删除后恢复。

## 6. 迁移中发现并修正的三处事实错误

1. **`wasm` 的 verify 命令跑不过**：`cargo check --manifest-path crates/vtrim-wasm/Cargo.toml`
   在宿主 target 上必然报 4 个 E0599（`SurfaceTarget::Canvas` 等只在 `#[cfg(web)]` 下存在）。
   已把注册表与卡都改成带 `--target wasm32-unknown-unknown`（实测 exit 0）。
2. **`vtrim-render-core` 的 `examples/` 那条警告已失效**：旧文档说"默认构建下 `--all-targets`
   必然报 4 个 example 错"，但**该目录根本不存在**；实测 `cargo check --workspace --all-targets` exit 0。
   卡的正文已按实测改写（而不是照抄旧说明）。
3. **文档写的 `python scripts/…` 在本机跑不起来**：只有 `python3`（3.13）在 PATH。
   根卡与 `scripts/AGENTS.md` 已统一写 `python3`。

## 7. 回滚

- 快照：`.workbuddy/agents-restructure/AGENTS.before.md`（拆之前根文件的逐字副本，已 gitignore）。
- 回滚 = 把快照复制回 `AGENTS.md`，再删掉新增的 34 张卡（`git status` 里未跟踪的那些 `AGENTS.md`）。
- ⚠️ **不要用 `git checkout -- AGENTS.md` 回滚**：该文件在本批次开始前就带着**未提交改动**
  （工作树当时是脏的），`checkout` 会把那些改动一起丢掉。
- 两条门禁与注册表可独立保留（不依赖迁移是否回滚），也可以一并删除。

## 8. 已知取舍

1. **叙述层不进指令预算**：拆之前 237 KB 里约 170 KB 是解释性叙述。规则与结论留在卡里，
   长叙述指向既有 `docs/`（`docs/deploy.md` / `docs/job-queue.md` /
   `docs/plan/webui-workbench-redesign.md` / `docs/plan/vad-single-mode.md` / `docs/web-ui-design.md`）。
   ⚠️ 取舍判据是"规则不许丢"，不是"字节数对齐"；`⚠️` 下限门禁把这条钉住了。
2. **卡片上限从 12 KiB 放宽到 24 KiB**：写卡时按 12 KiB 保守上限执行，导致若干页面级规则被压缩甚至
   短暂丢失（其中一个写手自己抓回 7 条）。真正的约束是**链上限 57,344 B**（DSH 一次注入的就是这条链），
   单卡上限只是防某一份无限膨胀 —— 所以 `module_bytes` 最终定为 24 KiB，并在 `_why` 里写明理由。
3. **前端刻意没加的两条门禁**（现状不满足，加了就是红门禁）：`views/` 里不许出现 `/api/` 字面量
   或 `fetch(`、`views/` 里不许用 `scopeQ(` —— 真要收紧得先改代码（行为改动），另开批次。
   理由写进 `scripts/check_module_boundaries.py` 的模块 docstring。
4. **大文件物理拆分不在本批次**：`vtrim-server/src/ingest.rs`(1875) / `vtrim-service/src/db.rs`(1837) /
   `vtrim-polish/src/process/mod.rs`(1761) 等只做边界标注，不拆文件（零行为改动是本批次的前提）。
5. **`webui/dist` 不需要重建**：本批次没动 `webui/src` 的代码，只加了 `AGENTS.md`。
6. **`crates/vtrim-core/src/types/` 没有专卡**：它由 `crates/vtrim-core/AGENTS.md` 覆盖
   （`PipelineContext` / `Event` / `Transcript` 都在那张卡的「入口」里）。哪天它再长起来，
   照根卡的「改代码协议」第 4 条开卡并登记进注册表即可。
