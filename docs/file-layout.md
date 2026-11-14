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
│   ├── server.toml               # 当前工程 + 端口等（账号无关）
│   │                             # ⚠️ 2026-11-13 起这里**没有** config.toml / llm.toml —— 见「程序配置只有一个位置」
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
（多用户下 `runtime/` 仍是**全局一份**：它是随版本发布的交付物，不是谁的素材。）

### 2026-10-10：按归属（`Owner`）—— 上面那张表是**单人形态**的形状

新增了**身份维度**（`yekitrim_core::owner`）：`Owner::Local`（托盘 / CLI / `--local-no-auth`）
与 `Owner::User(uid)`（多用户模式下某个账号）。**路径不再由调用方自己拼**，一律问 `owner`：

| 资产 | 单人（`Owner::Local`） | 多用户（`Owner::User(uid)`） |
|---|---|---|
| 素材根 | `<data>/resources/` | `<data>/users/<uid>/resources/` |
| 字体 / 贴纸 / 视频 | `<data>/resources/{fonts,stickers,videos}/` | `<data>/users/<uid>/resources/{fonts,stickers,videos}/` |
| 配置（`config.toml` / `llm.toml`） | `<data>/users/<首个管理员>/.yekitrim/`（**2026-11-13 起**；以前是顶层） | `<data>/users/<uid>/.yekitrim/` |
| **账本**（`users` / `sessions` / `invites` / `yekitrim.db`） | `<data>/yekitrim.db` | **同一份，永远全局** |

⚠️ **账本永远不按用户分** —— 它是**身份的来源**，按用户分就是"登录前先得知道你是谁"的循环依赖。
项目按 **行级** `projects.owner_id` 归属（本来就是对的形状，未改）。

素材那一列**逐字等于**上面那张老表 ⇒ 素材**单人升级零迁移**、目录形状不变。
判据：`owner::tests::local_owner_resolves_to_exactly_todays_locations`。

⚠️ **配置例外（2026-11-13，方案 A）**：程序配置**不再有第二个位置**。`Owner::Local` 也解析到
`users/<首个管理员>/.yekitrim/`；顶层 `<data>/.yekitrim/{config,llm}.toml` **已退役**，
启动时由 `config::migrate_top_level_config_to` 合并过去并把旧文件改名成 `*.migrated-<日期>.bak`。
起因：两份由同一模板生成、各被一半消费者读到（"设置保存成功、行为完全没变"）。
正文：`docs/plan/single-user-config.md`。**别再往顶层放配置** —— 那是这次要消灭的东西。

⚠️ **升级到多用户时，单人存量的归属**：`owner::migrate_legacy_assets_to` 把
`<data>/resources/*` 与 `<data>/.yekitrim/{config,llm}.toml` **移动**（不是复制 ——
素材可能几个 GB，复制会占双倍磁盘且两份从此分叉）到**首个管理员**（id 最小的 admin）名下。
幂等、目标已存在就跳过、`Owner::Local` 不参与、跨盘失败如实报错并保持原状。
启动时由 `yekitrim-server::migrate_solo_assets_to_first_admin` 跑一次。

⚠️ **渲染/封面/预览是子进程**（`yekitrim polish render|cover|preview`），拿不到父进程的
`Owner` ⇒ 父进程经 **`YEKITRIM_OWNER` 环境变量**传下去（`local` 或 uid），
子进程用 `Owner::from_env()` 读回。选环境变量而不是命令行参数：polish 每个子命令都要带上它、
且子进程还会拉起**孙进程**（ffmpeg / 底座），环境变量自动继承、命令行不会。
认不出的值回落 `Local` 并打 warn（**不报错** —— 出片失败比"用错了库"更难查）。

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

⚠️⚠️ **视频库是上面那条规矩的唯一例外，而且它是被真 bug 逼出来的**（2026-10-09 修）：
判据是"**有内容的**优先，都没有才看存在"。原因是用户手工搬迁的自然顺序是
**先建目录、后搬文件** —— 中途必然出现一个**空壳**：
```
resources/videos/            ← 新建的，还空着      ← 只看 is_dir() 就是它当选
resources/stickers/videos/   ← 搬迁中途留的空残留  ← 只看 is_dir() 也是它
stickers/videos/op.mp4       ← 真素材还在这儿      ← 于是永远轮不到
```
后果 = 视频库返回**空列表**，界面只说"库是空的"、**一个字都不报** ——
用户以为素材被删了。所以 `video_dir_readable_at` 走**两轮**：
① 挑"目录存在**且 `read_dir` 非空**"的；② 都没有才退回"目录存在"的那个
（全新安装本来就该是空的，那时返回新位置才对）。
判据：`tests/asset_layout_fallback.rs::empty_leftover_must_not_shadow_a_populated_library`
+ `all_empty_resolves_to_the_new_location`（已变异自证）。
⚠️ **贴纸库不适用这条**：贴纸包是**子目录**，`scan_packages` 本来就发现得了；
而且"空库"对贴纸是常态（用户还没建包），拿内容当判据会把新建的空贴纸库顶回老位置。

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
