# 目录结构（程序资产放在哪）

> 口径（2026-10-02 用户拍板）：**所有内容围绕程序目录来，不要用户目录**。
> 这份文档是那条口径的落地说明 —— 改路径解析之前先读它。

## 一个根：`data_dir()`

程序的全部可写资产（配置 / 账本 / 模型 / 字体 / 工程）都挂在**一个根**下面：

```
data_dir()  =  %VTRIM_DATA_DIR%  ??  exe_dir()
              （exe_dir = 可执行文件所在目录；取不到就退当前工作目录）
vtrim_home() =  data_dir()/.vtrim
```

- **debug 构建**：`exe_dir` = `target/debug/` → 资产也落在 `target/debug/`
- **发行版**：`exe_dir` = 安装目录 → 资产落在安装目录旁

⚠️ **`VTRIM_DATA_DIR` 是唯一的口子**，留着它解决一个具体问题：`target/debug` 与
`target/release`（以及换 `CARGO_TARGET_DIR`）各是一份程序目录，**1.19 GB 的模型会被复制两份**。
想共用就把它指到同一处：

```bash
set VTRIM_DATA_DIR=F:\vtrim-data
```

## 完整形状

```
<data_dir>/                       # 出厂 = 程序目录
├── vtrim.exe
├── .vtrim/
│   ├── config.toml               # 程序配置（全局默认，被 vtrim.toml 覆盖）
│   ├── llm.toml                  # LLM 配置（API key 在这里）
│   ├── server.toml               # 当前工程 + 端口等
│   ├── cloud/                    # ← 账号与运行态（db::data_dir）
│   │   ├── vtrim.db              # 账号账本（用户 / 会话 / 项目 / 审计 / 用量）
│   │   └── logs/                 # 运行日志
│   ├── projects/users/<uid>/     # 托管项目（[storage] projects_root 留空时的默认）
│   ├── models/                   # ← ASR 模型（asr::resolve_model_dir 的唯一落点）
│   │   ├── sensevoice/           #   single 模式打轴
│   │   ├── qwen3-asr/            #   hybrid 填词
│   │   └── silero-vad/           #   VAD
│   └── bin/                      # 内置下载器（yt-dlp）
├── resources/fonts/              # ← 字体库（path::font_dir 的唯一落点）
│   └── 放这里.txt
└── stickers/                     # 贴纸库（path::sticker_dir）
```

**只有这三处是"官方位置"**，且各自只有一份解析函数：

| 资产 | 唯一解析 | 落点 |
|---|---|---|
| 配置 / 账本 / 模型 / 工程 | `path::vtrim_home()` | `<data_dir>/.vtrim/` |
| 字体库 | `path::font_dir()` | `<data_dir>/resources/fonts/` |
| 贴纸库 | `path::sticker_dir()` | `<data_dir>/stickers/` |

### 不在这里的东西（有意）

- **工程素材**（录播视频）：在用户自己的盘上（例如 `E:\media\…`），
  由账本的 `projects.root_dir` 登记。素材不属于程序资产。
- **发行包**：`release/`（gitignored）—— 构建产物，不是运行态。
- **cargo 构建中间物**：`target/debug/{deps,build,incremental,.fingerprint}`。
  它们和程序资产**同处一个目录只是巧合**（都叫 target/debug），别把它们当成程序的一部分。

## 为什么会走到这一步（历史，别重蹈）

2026-10 之前，字体与模型**优先挂用户目录** `~/.vtrim/`，理由是躲开"换构建目录资产就没了"
（`target/cl/debug` 与 `target/debug` 各一份，下载过的东西全部"未下载"）。

代价是**同一个东西有了两个可能的位置**，于是必然有地方只查了一处。实测踩到两次：

1. **字体**：CLI 解析到 `target/debug/resources/fonts`，WebUI 解析到 `~/.vtrim/fonts` ——
   两边列出的字体**完全不同**，而界面上看不出任何异常。
2. **模型**：界面按 `<exe>/.vtrim/models` 判"未下载"，而 qwen3-asr 实际在 `~/.vtrim/models` ——
   显示"未下载"、转写却一直在用它。

现在位置只有一个，这类 bug 从根上没有发生的条件。**别再退回用户目录**：
"躲开构建目录漂移"这个诉求已经由 `VTRIM_DATA_DIR` 正面解决，不需要第二份资产。

### 用户目录里还剩什么

`~/.vtrim/` **已不再被读取**。老机器上可能还留着 `fonts/` / `models/` / `config.toml`，
程序启动时会**如实报出**旧字体库的路径与体积（`fonts::legacy_user_dir_report`），
但**不自动删也不搬** —— 那是用户的磁盘，里面可能有他手工放进去的字体。

确认没问题后手动删即可：

```powershell
Remove-Item -Recurse -Force "$env:USERPROFILE\.vtrim"
```
