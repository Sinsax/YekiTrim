# 标准个人目录结构（local ≡ 多人的 admin）

> 2026-10-10 定稿。本文是**布局的唯一权威**，`docs/file-layout.md` 保留历史与回落关系。
> 口径来源：用户拍板「**local = 多人的 admin，全按照多人模式进行**」。

## 一句话

**只有一种布局：多用户。** `Local`（托盘 / `--local-no-auth` / CLI）不再是"另一套目录"，
它**就是 admin 这个账号**，落在 `users/<admin-uid>/` 下。

```
<数据目录>/
├── .yekitrim/                      # 进程级、账号无关
│   ├── cloud/yekitrim.db           #   账本：users / sessions / invites / projects
│   ├── server.toml                 #   服务端记忆（上次工程、端口）
│   ├── models/                     #   ASR 模型（全机共享的大文件）
│   └── logs/                       #   运行日志
├── runtime/                        # 随版本发布的只读交付物（全机一份）
│   ├── polish-to-dhampir.mjs
│   └── effects.json
└── users/
    ├── 1/                          # ← admin（也就是 local 的那个人）
    │   ├── .yekitrim/
    │   │   ├── config.toml         #   程序配置（按账号）
    │   │   └── llm.toml            #   LLM 密钥（按账号）
    │   └── resources/
    │       ├── fonts/
    │       ├── stickers/
    │       └── videos/
    └── 2/                          # ← 普通账号，结构完全相同
        ├── .yekitrim/{config,llm}.toml
        └── resources/{fonts,stickers,videos}/
```

**没有 `resources/` 在顶层。** 顶层只留"进程级"与"随版本发布"这两类**账号无关**的东西。

## 谁是谁

| 概念 | 身份 | 数据位置 |
|---|---|---|
| 托盘 / `--local-no-auth` | **admin 本人**（合成会话，`LOCAL_SESSION_ID`） | `users/<首个管理员>/` |
| server 里 admin 登录 | **同一个人** | **同上**（与托盘**逐字节相同**） |
| 普通账号 | `User(uid)` | `users/<uid>/` |

⚠️ **托盘与"admin 登录"必须解析到同一处**。它们本来就是同一个人的两种进入方式 ——
以前托盘落 `resources/`、admin 登录落 `users/1/`，于是"同一个我在两个地方各有一套素材"，
这就是 2026-10-10 那次"贴纸构建了但没识别"的根：构建写进了 A，列表读的是 B。

## 账号无关的三类（刻意不按用户分）

| 东西 | 为什么不按用户分 |
|---|---|
| **账本**（`users`/`sessions`/`invites`/`projects`） | 它是**身份的来源**。"这个请求是谁"要先查它，按用户分就是循环依赖。 |
| **ASR 模型**（`.yekitrim/models/`） | 全机共享的大文件（几百 MB）；每人一份 = 磁盘翻倍、还得重下。 |
| **`runtime/`**（转译器 / 词汇表） | **随版本发布的交付物**，用户不该动，也不该按人存。 |

## 迁移（老 → 新）

单人时期的存量在 `<数据>/resources/` 与 `<数据>/.yekitrim/{config,llm}.toml`，
启动时**移动**（不是复制）到**首个管理员**（min-id 的 admin）名下：

```
<数据>/resources/*              →  <数据>/users/<第一个管理员>/resources/
<数据>/.yekitrim/config.toml    →  <数据>/users/<第一个管理员>/.yekitrim/
<数据>/.yekitrim/llm.toml       →  <数据>/users/<第一个管理员>/.yekitrim/
```

⚠️ **`rename` 而不是 copy**：素材可能几个 GB，复制会占双倍磁盘，而且两份从此分叉
——"构建写了一份、列表读另一份"正是分叉的症状。
⚠️ **幂等**：目标已存在就跳过；`users/` 目录**存不存在都不是判据**（它只在第一次按用户写过
东西时才出现，而升级后的第一件事就是启动 server —— 那一刻它必然不存在）。
判据只有一条：**源头有没有东西**。
⚠️ **跨盘失败如实报错**、保持原状（不要"尽力而为"地复制一半）。

## 与 2026-10-09 那轮"回落"的关系

`path::{sticker_dir_readable, video_dir_readable}` 的**老位置回落**是为 **`Local`** 写的，
它编码的是**这台机器的历史**（2026-10-09 之前 `resources/` 尚未收拢的那几种布局）。

新口径下 `Local` 已经等于 `User(admin)`，**用户树是新的、从来没有过"老位置"**
—— 所以 `User(uid)` 一律**不做回落**。给用户树加回落 = 凭空造出第二个候选位置，
正是本仓反复禁止的"候选列表式解析"。
