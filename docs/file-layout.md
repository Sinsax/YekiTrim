# 目录结构（程序资产放在哪）

> 口径（2026-10-02 用户拍板）：**所有内容围绕程序目录来，不要用户目录**。
> 这份文档是那条口径的落地说明 —— 改路径解析之前先读它。

## 一个根：`data_dir()`

程序的全部可写资产（配置 / 账本 / 模型 / 字体 / 工程）都挂在**一个根**下面：

```
data_dir()  =  %YEKITRIM_DATA_DIR%  ??  exe_dir()
              （exe_dir = 可执行文件所在目录；取不到就退当前工作目录）
yekitrim_home() =  data_dir()/.yekitrim
```

- **debug 构建**：`exe_dir` = `target/debug/` → 资产也落在 `target/debug/`
- **发行版**：`exe_dir` = 安装目录 → 资产落在安装目录旁

⚠️ **`YEKITRIM_DATA_DIR` 是唯一的口子**，留着它解决一个具体问题：`target/debug` 与
`target/release`（以及换 `CARGO_TARGET_DIR`）各是一份程序目录，**1.19 GB 的模型会被复制两份**。
想共用就把它指到同一处：

```bash
set YEKITRIM_DATA_DIR=F:\yekitrim-data
```

## 完整形状

```
<data_dir>/                       # 出厂 = 程序目录
├── yekitrim.exe
├── .yekitrim/
│   ├── config.toml               # 程序配置（全局默认，被 yekitrim.toml 覆盖）
│   ├── llm.toml                  # LLM 配置（API key 在这里）
│   ├── server.toml               # 当前工程 + 端口等
│   ├── cloud/                    # ← 账号与运行态（db::data_dir）
│   │   ├── yekitrim.db              # 账号账本（用户 / 会话 / 项目 / 审计 / 用量）
│   │   └── logs/                 # 运行日志
│   ├── projects/users/<uid>/     # 托管项目（[storage] projects_root 留空时的默认）
│   ├── models/                   # ← ASR 模型（asr::resolve_model_dir 的唯一落点）
│   │   ├── sensevoice/           #   single 模式打轴
│   │   ├── qwen3-asr/            #   hybrid 填词
│   │   └── silero-vad/           #   VAD
│   └── bin/                      # 内置下载器（yt-dlp）
├── resources/                    # ← 用户素材库的根（path::resources_dir）
│   ├── fonts/                    # 字体库（path::font_dir 的唯一落点）
│   │   └── 放这里.txt
│   ├── stickers/                 # 贴纸库（path::sticker_dir）
│   └── videos/                   # 视频素材库（path::video_dir —— 片头/片尾从这里选）
└── runtime/                      # ← 程序自带的交付物（path::runtime_dir，只读、用户别动）
    ├── polish-to-dhampir.mjs     #   转译器（本仓方言 → 底座 Dhampir 工程）
    └── effects.json              #   效果词汇表（11 个效果类型的唯一权威）
```

**只有这四类"官方位置"**，且各自只有一份解析函数：

| 资产 | 唯一解析 | 落点 |
|---|---|---|
| 配置 / 账本 / 模型 / 工程 | `path::yekitrim_home()` | `<data_dir>/.yekitrim/` |
| 字体库 | `path::font_dir()` | `<data_dir>/resources/fonts/` |
| 贴纸库 | `path::sticker_dir()` | `<data_dir>/resources/stickers/` |
| 视频素材库 | `path::video_dir()` | `<data_dir>/resources/videos/` |
| 转译器 + 词汇表 | `path::runtime_dir()` | `<data_dir>/runtime/` |

⚠️ **`resources/` 与 `runtime/` 的分界是"谁往里放东西"**：`resources/` 是**用户素材**
（用户增删贴纸/视频/字体，`放这里.txt` 就是写给用户的），`runtime/` 是**随版本发布的只读交付物**
（用户不该动）。**发布包只装 `runtime/`**，`resources/` 只建空目录 + 说明。

### 2026-10-09 的收拢（与老位置的回落关系）

这一轮把"同类资产住两个不同名字的根"合成一棵树，并顺手消掉一个隐患：

| 资产 | 老位置（**仍读**） | 新位置（**写**） |
|---|---|---|
| 贴纸库 | `<data_dir>/stickers/` | `<data_dir>/resources/stickers/` |
| 视频库 | `<data_dir>/stickers/videos/`（更早）<br>`<data_dir>/resources/stickers/videos/`（中间态） | `<data_dir>/resources/videos/` |
| 转译器 | `<data_dir>/tools/polish-to-dhampir.mjs` | `<data_dir>/runtime/polish-to-dhampir.mjs` |
| 词汇表 | `<data_dir>/spec/effects.json` | `<data_dir>/runtime/effects.json` |

⚠️ **写一律往新位置，读两个位置都认**（`sticker_dir_readable()` / `video_dir_readable()`），
判据是**目录存在与否**、不是"里面有没有东西" —— 用内容当判据会让"新建的空库"被老位置顶掉。
**不做自动搬迁**（用户手工移）；回落的意义只是"升级后库不会突然变空"。
判据：`path::tests::readable_dirs_fall_back_to_legacy_locations`。

⚠️ **视频库挪出 `stickers/` 的直接收益**：它原先是个普通子目录，而 `scan_packages` 会把
"非 `.` 开头、非 `backup`"的子目录都当贴纸包扫 —— 装满 mp4 时它以"一张图都没有"被丢掉，
**今天不污染列表纯属巧合**（`if !files.is_empty()`）。挪开之后这条从"碰巧不发生"
变成"结构上不可能"。

⚠️ **仓库源码树不受影响**：`tools/` 与 `spec/` 在**仓库里**仍是原名原有位置
（`tools/` 有模块卡与注册表登记，`spec/effects.json` 是权威源）。变的是**发布包与运行时的落点**。
转译器的词汇表查找因此要试**两档**：同目录 `./effects.json`（新发布包）与
`../spec/effects.json`（开发树 / 老包）。

### 不在这里的东西（有意）

- **工程素材**（录播视频）：在用户自己的盘上（例如 `E:\media\…`），
  由账本的 `projects.root_dir` 登记。素材不属于程序资产。
- **发行包**：`release/`（gitignored）—— 构建产物，不是运行态。
- **cargo 构建中间物**：`target/debug/{deps,build,incremental,.fingerprint}`。
  它们和程序资产**同处一个目录只是巧合**（都叫 target/debug），别把它们当成程序的一部分。

## 为什么会走到这一步（历史，别重蹈）

2026-10 之前，字体与模型**优先挂用户目录** `~/.yekitrim/`，理由是躲开"换构建目录资产就没了"
（`target/cl/debug` 与 `target/debug` 各一份，下载过的东西全部"未下载"）。

代价是**同一个东西有了两个可能的位置**，于是必然有地方只查了一处。实测踩到两次：

1. **字体**：CLI 解析到 `target/debug/resources/fonts`，WebUI 解析到 `~/.yekitrim/fonts` ——
   两边列出的字体**完全不同**，而界面上看不出任何异常。
2. **模型**：界面按 `<exe>/.yekitrim/models` 判"未下载"，而 qwen3-asr 实际在 `~/.yekitrim/models` ——
   显示"未下载"、转写却一直在用它。

现在位置只有一个，这类 bug 从根上没有发生的条件。**别再退回用户目录**：
"躲开构建目录漂移"这个诉求已经由 `YEKITRIM_DATA_DIR` 正面解决，不需要第二份资产。

### 用户目录里还剩什么

`~/.yekitrim/` **已不再被读取**。老机器上可能还留着 `fonts/` / `models/` / `config.toml`，
程序启动时会**如实报出**旧字体库的路径与体积（`fonts::legacy_user_dir_report`），
但**不自动删也不搬** —— 那是用户的磁盘，里面可能有他手工放进去的字体。

确认没问题后手动删即可：

```powershell
Remove-Item -Recurse -Force "$env:USERPROFILE\.yekitrim"
```
