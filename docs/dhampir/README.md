# Dhampir 承接：文档与工具（V-Trim 侧）

这一目录是 **V-Trim ↔ Dhampir（底座）** 承接关系的**唯一落点**。

> **迁移说明**：下列文件于 2026-09-30 从 **Dhampir 仓** 迁来（当时 Dhampir HEAD `8e98abf`），
> 源文件已从那边删除 —— 底座仓从此只放"底座自己的性质"，不再放下游方言的内容。
> 逐字节对账过 SHA256，迁移时未改内容；本目录里只有本 README 与 `scripts/vtrim-compare.mjs`
> 的路径解析是新的。

## 迁移映射

| 现在（本仓） | 原位置（Dhampir 仓） |
|---|---|
| `docs/dhampir/polish-translation.md` | `plan/vtrim-polish-translation.md` |
| `docs/dhampir/gap-plan.md` | `plan/vtrim-gap-plan.md` |
| `docs/dhampir/next-stage-report.md` | `plan/vtrim-next-stage-report.md` |
| `docs/dhampir/compare-loop.md` | `docs/vtrim-integration.md`（Dhampir 侧的 §9 对比回路 + §10 边界） |
| `scripts/vtrim-compare.mjs` | `scripts/vtrim-compare.mjs`（路径解析已改成 V-Trim 视角） |
| `docs/dhampir/p9-downstream-requirements.md` | `plan/p9-polish-parity-design.md` 的 §2 / §7 / §9（底座设计那几节留在 Dhampir） |

## 这几份各是什么

| 文件 | 是什么 |
|---|---|
| `polish-translation.md` | `polish.toml → 底座工程文件` 的**转译实测记录**：每一处缺口、每一处实测差、每一次改口径 |
| `gap-plan.md` | 由转译报出的**缺口总表与分阶段计划**（阶段 0–5 已收口；§1 的缺口表是"还差什么"的权威清单） |
| `next-stage-report.md` | 与 V-Trim 的**差异 · 性能 · 基座性质**实测报告（§1 性能头对头、§2 Warp 级静默失效、§3 效果残差、§4 三个结构性问题、§5 下阶段计划） |
| `compare-loop.md` | 对比回路怎么用：从哪拿转译器、`vtrim-compare.mjs` 干什么、`--verdict` 通道、底座侧的边界承诺 |
| `p9-downstream-requirements.md` | P9 的**下游需求来源**：V-Trim polish 的 15 个事件变体 / 样式表 / 布局表，以及"什么归底座、什么归 V-Trim"的分工表 |
| `base-handoff.md` | 2026-10-01 交给底座的**问题交接单**（D1–D4 / B1–B5，含最小复现与验收判据）。**处置结果记在 Dhampir 仓的 `plan/dhampir-base-handoff.md`**（那份已被底座中性化，只留"下游"的说法） |
| `../scripts/vtrim-compare.mjs` | 一步跑完整条回路的对比工具（转译 → 两边出片 → 按事件窗口出表） |
| `../plan/dhampir-animation-handoff.md` | **动图（GIF / 动画 WebP）原生解码的接入说明**（底座 host_api **6**）：两条新导出的契约、要改的三处调用点、「`sources_for` 会照旧包含动图层」这个坑、显存按原生尺寸算这条约束，以及**那一版没有验的**（浏览器侧未跑真素材 / WebP 无真实素材）。放在 `docs/plan/` 而不是这里，是因为它是**接入动作清单**，与本目录那几份"承接关系说明"性质不同 |

## 边界（一句话）

**V-Trim 决定"画什么"，Dhampir 决定"怎么画出来，且预览与成片画得一样"。**
底座对外承诺的只有 Dhampir 仓的 `docs/api-surface.md` 与 `docs/host-api.md`，
以及产物 `VERSION` 里的**两条版本线**：

* `project_schema` —— 工程文件能不能读（动图这一版**没动**，仍是 1）；
* `host_api` —— wasm 导出面/形状对不对得上（动图这一版 **5 → 6**）。

**启动时两条都要校验。** 只查 `project_schema` 会漏掉「工程能读、但少了两条导出」那种错配 ——
那时对端拿到的是 `undefined`，而不是一个说得清的错误。

**转译器不承诺兼容**，形状可以随本仓演化。

## 怎么跑对比回路

```bash
# 只转译（不跑对比）——本仓自带，零依赖
node tools/polish-to-dhampir.mjs "<片段目录>" --out target/vtrim/t.json

# 整条回路（需要：PATH 上有 ffmpeg/ffprobe、Dhampir 的可执行、vtrim 可执行、字体）
node scripts/vtrim-compare.mjs --clip "<片段目录>" --events <事件.json> --out target/vtrim-run

# 只跑自检（不碰工程、不需要上面任何外部依赖）
node scripts/vtrim-compare.mjs --self-test
```

底座可执行按候选列表找：`--dhampir` → `VTEDIT_DHAMPIR` → 同层 `Dhampir/target/{debug,release}/`。
转译器与 `vtrim` 也是同一套办法（`--translator` / `VTRIM_TRANSLATOR`、`--vtrim` / `VTRIM_EXE`）。

## 还没做的

* `scripts/vtrim-compare.mjs` **没进本仓的守卫清单**（本仓的守卫是 `scripts/check-*.mjs` 那一组）；
  它自带 `--self-test`，但没有人替你定期跑。
* 上面这份 README 是一张地图，**不是结论**；数字以各文档里的原始读数与命令为准。
