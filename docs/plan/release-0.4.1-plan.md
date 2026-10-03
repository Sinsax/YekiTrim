# v0.4.1 发行计划（待发）

| 项 | 值 |
|---|---|
| 目标版本 | **v0.4.1**（补丁版；上一版 v0.4.0，2026-10-03） |
| 状态 | ⏸ **未发布** —— 只登记计划，等要发时再执行 |
| 主题 | WebUI 换肤（品牌粉 → 砖红 + 沙/驼）+ 新品牌标 |
| 平台 | Windows x64 |

## 为什么是补丁版号而不是 0.4.0 重打包

用户拍板（2026-10-03）：**不重新打 v0.4.0 的包**，换肤作为 0.4.1 发布。

- `release/yekitrim-v0.4.0-win-x64.zip`（已发到 Gitea）里是**换肤前**的粉色界面，**保持原样不动**。
  发行包一旦发布就不再改动 —— 改了 hash 就对不上已公布的 `.sha256.txt`。
- 换肤**不改任何对外契约**（接口/配置/数据格式/CLI 参数全没动，CSS 变量**名**也没改），
  所以补丁版号在语义上说得通。

> ⚠️ 历史记录订正：`a6b7d86` 当初写成 `feat(webui)!:`，那个 `!` **是错的**，
> 已由 `d3b6462` 空提交订正。该提交已推送，没做 force-push 改写共享历史。

## 本版包含的改动（`v0.4.0..HEAD`）

| 提交 | 内容 |
|---|---|
| `a6b7d86` | 换肤：色板 + 品牌标 + 清掉 9 处绕过 tokens 的硬编码 |
| `ba5cda8` | 文档：Gitea 发布纪律 + 两处遗留的解决结果 |
| `d3b6462` | 文档：订正换肤提交的类型标记 |

## 换肤要点（发布说明素材）

**三个参考色**（用户给定）：`#cab5a0` 沙 / `#cba98e` 驼 / `#d86353` 砖红。
**角色分配**：砖红当主色，沙/驼当底色与辅助。

⚠️ **为什么不能反过来**：实测沙色/驼色**压白字对比度只有 1.98 / 2.19**（WCAG 图形需 3.0），
做按钮底色会让白字糊掉；只有砖红能到 **3.60**。

| 角色 | v0.4.0 | **v0.4.1** |
|---|---|---|
| `--main` | `#F682B8` | **`#d86353`** |
| `--main-deep` | `#EE6FA6` | **`#B15144`**（暗化 0.82，文字色专用；压暖白 **4.81 过 AA**） |
| `--main-soft` | `#FDE3EC` | `#FAECEA` |
| `--main-glow` | `#FFD3E1` | `#F5D6D2` |
| `--bg` | `#FFF7F3` | `#F7F3F0` |
| `--bg-soft` | `#FFEFE7` | `#F0EAE4` |
| `--line` / `--line-soft` | `#F2E1D9` / `#FAEEE7` | `#E7DED4` / `#EFE9E2` |
| `--ink` | `#4A3F45` | `#574A40` |
| `--ink-dim` | `#9C8B92` | **`#77685B`**（原值压暖白仅 2.6，**不达标**，顺手修） |
| `--ink-faint` | `#C9B8BE` | `#A79483` |
| 新增 | — | `--tan` `#cba98e` / `--tan-soft` `#F6EFE8` / `--tan-deep` `#8C6F57` |

⚠️ **三个变量名保留但语义已变**（改名会牵动 27+ 处引用，风险大于收益）：
`--pink-light`（现=砖红浅调）/ `--pink-text`（现=砖红文字 = `--main-deep`）/
`--shadow-pink`（现=砖红投影）。

**品牌标**（新增，此前只有字母「Y」）：暖白圆角方 + 砖红 Y + 沙色下竖（切片意象）。
favicon 与登录页/侧栏用同一套 SVG 几何；圆形容器渐变 `--main → --pink-light` 改为 `--main → --tan`。

**顺带清掉的 9 处硬编码**（这类值绕过 `tokens.css`，是换肤必漏点）：

- 旧粉三种写法：`rgba(238,111,166,·)` / `rgba(246,130,184,·)` / `#f682b84d`
- 旧**冷褐**中性色：`rgba(90,60,70,·)` / `rgba(74,63,69,·)` / `#4a3f4552`
- `rgba(255,247,243,.85)`（`App.vue` 吸顶面包屑底）、`#3b2f36`/`#6b4a58`（`.player` 渐变）

## 发布时要做的事（检查清单）

1. `Cargo.toml` 的 `[workspace.package] version` → `0.4.1`（7 个 crate 靠 `version.workspace = true` 继承）
2. 写 `docs/releases/v0.4.1.md`（素材见上）
3. `cargo check --workspace --all-targets`（0 warning）+ `cargo test --workspace`（基线 759 passed）
4. 两条门禁 + `npm --prefix webui run build`（确认 dist 是最新）+ 8 个前端契约脚本
5. `cargo xtask package --smoke-test`（含 espeak-ng 产物门禁 + 冒烟）
6. ⚠️ **核对包内界面确实是新皮肤**（v0.4.0 的教训：包是换肤前打的）
7. `git tag -a v0.4.1` → push tag → 在 Gitea 建 Release 并传 zip + `.sha256.txt`

> ⚠️ **Gitea 发布纪律**（见 `rename-execution-plan.md`「Gitea 发布纪律」）：
> Release 列表按 **tag 的 tagger 时间**排序；`PATCH /releases/{id}` **改不动时间**，
> 唯一有效修法是重建 tag（`GIT_COMMITTER_DATE`）再重建 Release。
