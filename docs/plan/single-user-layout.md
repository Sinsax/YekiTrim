# 标准目录结构（local ≡ 多人的 admin + 全局公用层）

> 2026-10-10 定稿。本文是**布局的唯一权威**，`docs/file-layout.md` 保留历史与回落关系。
> 口径来源：用户拍板「**local = 多人的 admin，全按照多人模式进行**」+「**增加一个 global 全局目录**」。

## 一句话

**三层：全局（公用）+ 用户（私有）。没有"程序目录那套"。**

```text
<数据目录>/                          ← 默认 = exe 同级；YEKITRIM_DATA_DIR 可覆盖
├── .yekitrim/                      【账号无关】
│   ├── cloud/yekitrim.db           账本：users / sessions / invites / projects
│   ├── server.toml                 服务端记忆（上次工程、端口）
│   ├── models/                     ASR 模型（全机共享的大文件）
│   └── logs/
├── runtime/                        【账号无关】随版本发布的只读交付物
│   ├── polish-to-dhampir.mjs
│   └── effects.json
├── global/                         【所有人的公用层】
│   ├── .yekitrim/
│   │   ├── config.toml             配置**初始化模板**
│   │   └── llm.toml                LLM 密钥模板
│   └── resources/
│       ├── fonts/                  公用字体（含程序自带的 2 支）
│       ├── stickers/               公用贴纸包
│       └── videos/                 公用视频素材
└── users/
    ├── 1/                          ← admin（也就是 local 那个人）
    │   ├── .yekitrim/
    │   │   ├── config.toml         自己的配置（首次从 global 拷一份）
    │   │   └── llm.toml
    │   └── resources/{fonts,stickers,videos}/
    └── 2/                          ← 普通账号，结构完全相同
```

⚠️ **顶层没有 `resources/`。** 它以前是"单人的素材库"，现在那套彻底没有了 ——
素材只有"公用（global）"与"某个账号的（users/<uid>）"两种。

## 谁是谁

| 场景 | 身份 | 数据位置 |
|---|---|---|
| 托盘 / `--local-no-auth` / CLI | **admin 本人** | `users/<admin>/` |
| server 里 admin 登录 | **同一个人** | **同上（逐字节相同）** |
| 普通账号 | `User(uid)` | `users/<uid>/` |

## 素材库：共享内容，但**写入永远只落用户库**

**读**按优先级查两层（`owner::asset_dirs(owner, kind)` = `[用户库, 全局库]`）：

1. 用户库里有这个名字 → **用它**（用户能覆盖公用的同名素材）；
2. 没有 → 去全局库找；
3. 都没有 → 就是没有。

**写**只有一处：`owner::asset_dir(owner, kind)` = **用户库**。
导出的、构建出来的、上传的，一律进自己的库。

> ⚠️ **别把写入也改成"往列表里第一个存在的目录写"**：那会让"全局"变成
> "谁先写谁占用"，公用素材被某个账号的私有版本顶掉，而其他人看到的东西随之变化。

## 配置：`global` 那份是**初始化模板**，不是"每次合并的默认值"

账号**首次初始化**时，从 `global/.yekitrim/{config,llm}.toml` **拷一份**到
`users/<uid>/.yekitrim/`；之后各自独立演进。

**为什么不是"读取时逐键合并"**：那样改一次全局配置会**改变所有还没覆盖那一键的人**的
行为 —— 用户会觉得"我的设置自己变了"；而"我明明改了自己那份"也可能被全局悄悄盖掉。
**初始化一次**的语义是可预期的：拷完之后，你的就是你自己的。

⚠️ 幂等：**目标已存在就跳过**，绝不覆盖（覆盖会冲掉用户改过的 API key / 检测参数）。
⚠️ 全局那份也没有时不报错 —— 走各处的内置默认值即可（全新安装就是这样）。

## 顺序（三条，错了都**不报错**、只是结果不对）

server 启动时：`install_local_admin` → 存量迁移 → `ensure_global_config` → `seed_account_config`

| 步骤 | 必须在此之前 | 反了的后果 |
|---|---|---|
| `install_local_admin` | 一切按 `Local` 取路径的操作 | 落到默认 `users/1/`（通常恰好对，故只在"首个管理员不是 1"时露馅） |
| 存量迁移 | `seed_account_config` | 先 seed 会给管理员建一份"出厂默认"，把用户改过的那份挡在门外 |
| `ensure_global_config` | `seed_account_config` | 模板还没生成，seed 找不到源、静默不做（实测：新库启动后 `users/1` 下空空如也） |

## 账号无关的三类（刻意不按用户分）

| 东西 | 为什么不按用户分 |
|---|---|
| **账本**（`users`/`sessions`/`invites`/`projects`） | 它是**身份的来源**。"这个请求是谁"要先查它，按用户分就是循环依赖。 |
| **ASR 模型**（`.yekitrim/models/`） | 全机共享的大文件（几百 MB）；每人一份 = 磁盘翻倍、还得重下。 |
| **`runtime/`**（转译器 / 词汇表） | **随版本发布的交付物**，用户不该动，也不该按人存。 |

## 自带字体落在 `global/`

程序自带的 2 支（Noto Sans SC / Noto Color Emoji，约 20 MB）装在 `global/resources/fonts/`：
它们是"程序自带的**公用**素材"，不属于任何账号 —— 装一份所有人共用，
而不是每个账号首次启动各落一份（20 MB × 人数）。

## 迁移（老 → 新）

单人时期的存量在 `<数据>/resources/` 与 `<数据>/.yekitrim/{config,llm}.toml`，
启动时**移动**到**首个管理员**（min-id 的 admin）名下：

```
<数据>/resources/*              →  <数据>/users/<第一个管理员>/resources/
<数据>/.yekitrim/config.toml    →  <数据>/users/<第一个管理员>/.yekitrim/
<数据>/.yekitrim/llm.toml       →  <数据>/users/<第一个管理员>/.yekitrim/
```

⚠️ **`rename` 而不是 copy**：素材可能几个 GB，复制会占双倍磁盘，而且两份从此分叉
——"构建写了一份、列表读另一份"正是分叉的症状。
⚠️ **幂等**；判据只有一条：**源头有没有东西**（不看 `users/` 在不在）。
⚠️ **跨盘失败如实报错**、保持原状（不要"尽力而为"地复制一半）。

## 与 2026-10-09 那轮"回落"的关系

`path::{sticker_dir_readable, video_dir_readable}` 的**老位置回落**是给 2026-10-09 之前
那几种布局写的。新方案下素材只有"用户库 + 全局库"两层，**两层都查、顺序固定** ——
用户的树是新的、从来没有过"老位置"，所以 `User(uid)` 一律**不做回落**。
给用户树加回落 = 凭空造出第二个候选位置，正是本仓反复禁止的"候选列表式解析"。
