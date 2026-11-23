#!/usr/bin/env python3
r"""模块边界门禁：把"是否嵌合"变成可执行的判断，而不是靠感觉。

为什么要有这个脚本
------------------
两类**硬边界**写成散文就会腐坏 —— 一开始没人违反，后来某次"就这一次"加个 import
就破防了，而且不会报错、测试也不会红。所以这里把它变成一个本地/CI 都能跑的检查：

1. **P4（B站链接通道）的隔离**（见 `docs/plan/p4-implementation-plan.md` §2）：
   谁不许认识谁、谁不许开子进程。
2. **分层方向与前端边界**（见根 `AGENTS.md`「架构」与各模块卡的「边界」小节）：
   `core` / `polish` / `render-core` 不许认识上层；`yekitrim-service` 不许 `use yekitrim_server`
   （这条以前只写在 `yekitrim-service/src/lib.rs` 的模块注释里，没有任何机器判据）；
   `yekitrim-server` 不许碰 `yekitrim-cli`；CLI 只经 `yekitrim_server::{run, ServerArgs}` 进服务端；
   前端 `components/` 不许 import `views/`；作用域查询串只许在 `webui/src/api/` 里拼。

判据为什么长这样（几条豁免，都是踩过的坑）
------------------------------------------
1. **先剥注释再匹配**：规则说明自己就会写下"本目录不许出现 `yt-dlp`"，不剥注释的话
   这些文档行会全部命中 —— 上一版手写的 grep 判据就因此产出 8 条假阳性，等于没用。
   `(?<!:)//` 是为了不把 `https://…` 里的 `//` 当注释起点。
2. **`std::process::id()` 不算"开子进程"**：它只取 PID 给临时目录命名
   （`downloader/ytdlp.rs`、`bilibili/danmaku.rs` 都用了），没有任何进程被创建。
   所以规则认的是 `Command` / `.spawn(`，不是笼统的 `std::process`。
3. **分层规则用 `(?<!["\w])` 而不是裸 crate 名**：`yekitrim-service/src/logbus.rs` 有一段测试
   专门断言"三个 crate 前缀都要剥掉"，里面写着字符串字面量 `"yekitrim_server::"` ——
   那是数据不是依赖，裸匹配会误报。所以要求 crate 名前面既不是引号也不是标识符字符。
4. **CLI 那条只放行 `run` / `ServerArgs`**：`yekitrim-cli` 本来就依赖 `yekitrim-core` /
   `yekitrim-polish`（入口装配要用），但**服务端能力**只能经 `yekitrim_server::run()` /
   `ServerArgs` 进，不许直接调 server 的内部模块。
5. **"子进程不许绕过 core 的启动助手"有一条例外机制**：规则里用定长后行断言
   `(?<!tokio::process::)` 放过 tokio 那份（它自己走 `as_std_mut`），另用 `EXEMPT`
   表放过**规则实现本身**（`core/src/process.rs`）与**薄启动器**（`cli/src/tray.rs`：
   它连 core 都不能链接，否则 ASR 会被拉回来，所以只能自己设 `CREATE_NO_WINDOW`）。
   `EXEMPT` 只给这两类开口子，别拿来放业务代码 —— 那等于把不变式悄悄作废。
6. **前端刻意没加的两条**（现状不满足，加了就是红门禁，别盲加）：
   `views/` 里不许出现 `/api/` 字面量或 `fetch(` —— 现状大量直接 `api('/api/…')`；
   `views/` 里不许用 `scopeQ(` —— `ProjectSettingsView` / `PolishView` / `router.js` 都在用。
   真要收紧得先改代码，那是行为改动，不属于"结构标注"这一批。

用法
----
    python3 scripts/check_module_boundaries.py        # 违规非零退出
    python3 scripts/check_module_boundaries.py -v     # 同时列出通过项

⚠️ 用 `python3`：本机（以及不少 Windows 环境）`python` 并不在 PATH 上。
"""

from __future__ import annotations

import re
import subprocess
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent

# (规则名, 相对路径列表, 正则, 为什么)
RULES: list[tuple[str, list[str], str, str]] = [
    (
        "downloader 不许开子进程",
        ["crates/yekitrim-service/src/downloader"],
        r"process::Command|\bCommand::new\b|\.spawn\(",
        "执行只在 yekitrim-server/download_worker.rs；适配层要能离线可测",
    ),
    (
        "downloader 不许知道 B站",
        ["crates/yekitrim-service/src/downloader"],
        r"\bbvid\b|\bcid\b|bilibili\.com|comment\.bilibili",
        "引擎是通用端口；沾上 B站就只能适配一家，加第二个引擎要改这个目录",
    ),
    (
        "bilibili 不许知道下载器",
        ["crates/yekitrim-service/src/bilibili"],
        # ⚠️ 哨兵名 2026-10-03 由 `YEKITRIM_*` 改成 `YEKITRIM_*`（改名 yekitrim → yekitrim）。
        # **两个名字都留在正则里**：这个规则是"判据本身"，改名时若只改代码不改这里，
        # 规则会**静默匹配不到任何东西**（门禁还在跑、还报 0 违规，但其实已经是空转）。
        # 实测过：注入 `YEKITRIM_PATH` 旧正则会漏（exit 0），注入 `YEKITRIM_PATH` 才抓得到（exit 1）。
        r"yt-?dlp|YEKITRIM_PROG|YEKITRIM_PATH|YEKITRIM_META"
        r"|YEKITRIM_PROG|YEKITRIM_PATH|YEKITRIM_META|process::Command|\.spawn\(",
        "弹幕走旁路（自己 GET），与引擎完全无关；混在一起就没法单测",
    ),
    (
        "net/ 不许知道 B站",
        ["crates/yekitrim-service/src/net"],
        r"\bbvid\b|\bcid\b|bilibili\.com",
        "deflate 解码是通用工具，不该知道调用方是谁",
    ),
    (
        "download_worker 不许引用 ingest_worker",
        ["crates/yekitrim-server/src/download_worker.rs"],
        r"\bingest_worker\b",
        "两个 worker 并列；共享部分走 ingest::{capacity,landing} 与 pipeline_boot",
    ),
    (
        "download_worker 不许有录播姬语义",
        ["crates/yekitrim-server/src/download_worker.rs"],
        r"is_finalized|move_file\(",
        "录播姬靠空闲窗口猜完结、从录像目录搬入；B站进程退出码就是答案，直接写进托管目录",
    ),
    # ---- 分层方向（见根 AGENTS.md「架构」；每条都对应一张模块卡的「边界」小节）----
    (
        "core 不许认识上层",
        ["crates/yekitrim-core/src"],
        r"yekitrim_polish|yekitrim_service|yekitrim_server|yekitrim_cli",
        "分层是单向的 core → polish → service → server → cli；底层一旦反向引用，两层就粘回去",
    ),
    (
        "render-core 不许认识上层",
        ["crates/yekitrim-render-core/src"],
        r"yekitrim_polish|yekitrim_service|yekitrim_server|yekitrim_cli",
        "绘制原语只被 polish / wasm 用；它不认识业务",
    ),
    (
        "polish 不许认识 service/server/cli",
        ["crates/yekitrim-polish/src"],
        r"yekitrim_service|yekitrim_server|yekitrim_cli",
        "polish 只依赖 core + render-core；要领域能力就必须经调用方注入",
    ),
    (
        "service 不许认识 server",
        ["crates/yekitrim-service/src"],
        r'(?<!["\w])yekitrim_server::',
        "`lib.rs` 的明文不变式（它能被抽出来的全部理由），以前没有机器判据",
    ),
    (
        "server 不许认识 cli",
        ["crates/yekitrim-server/src"],
        r'(?<!["\w])yekitrim_cli::',
        "server 只依赖 service；入口在 cli 那一侧",
    ),
    (
        # ⚠️ 判据要**排除 tokio 那份**（`tokio::process::Command::new`）：它自己另走
        # `hide_console_window(cmd.as_std_mut())`。用定长后行断言排除，别把正则放宽成"全都放过"。
        "子进程不许绕过 core 的启动助手",
        [
            "crates/yekitrim-core/src",
            "crates/yekitrim-polish/src",
            "crates/yekitrim-service/src",
            "crates/yekitrim-server/src",
            "crates/yekitrim-cli/src",
        ],
        r'(?<!tokio::process::)(?<!["\w])Command::new\(',
        "托盘是无控制台子系统（windows_subsystem = windows），直接 Command::new 会让每个控制台子进程"
        "自己新开一个控制台窗口 —— 用户看到的就是「启动时黑框一闪」（实测启动那两个 ffmpeg/ffprobe"
        "探测就冒了两个 ConsoleWindowClass）。一律走 `yekitrim_core::process::cmd()`",
    ),
    (
        "cli 只用 server 的入口",
        ["crates/yekitrim-cli/src"],
        r'(?<!["\w])yekitrim_server::(?!(run|ServerArgs))',
        "服务端能力只能经 yekitrim_server::run() / ServerArgs 进；不许直接调 server 内部模块",
    ),
    # ---- LLM 隔离（P3；见 crates/yekitrim-llm/AGENTS.md「本 crate 有两条专属门禁」）----
    (
        "llm_is_a_leaf",
        ["crates/yekitrim-llm/src"],
        r"yekitrim_core|yekitrim_polish|yekitrim_service|toml::",
        "LLM 传输层是**叶子 crate**：它只吃 LlmParams —— 不认识配置文件、不认识任何 yekitrim-* crate。"
        "配置 → 参数的换算在 yekitrim-core（impl From<&LlmConfig> for LlmParams）；"
        "它一旦 use 上层或自己读 TOML，依赖方向就成环、「隔离」只剩散文里那句话",
    ),
    (
        # **扫整棵 crates/，唯一的例外是规则自己点名的那个 crate**（见 EXEMPT）。
        # 不写成"逐个列举其它 crate"是有意的：新加一个 crate 时会**自动**落进判据里，
        # 而列举法会让它静默逃过（漏一条 = 那条不变式对新代码不存在）。
        "only_llm_knows_wire",
        ["crates"],
        r"reasoning_effort|enable_thinking|max_completion_tokens|chat_template_kwargs",
        "wire 字面量（推理参数怎么拼、输出上限发哪个字段名）**只许出现在 yekitrim-llm**："
        "业务侧只表达「要什么」（LlmParams / ModelCaps），怎么发由 wire.rs 一处决定。"
        "散出去就是「同一个旋钮在两处各写一半」—— 从前 [llm] temperature 在 build_body 写死、"
        "extra_params 再盖一层、thinking 再盖第三层。"
        "刻意**不**收 response_format / thinking：core 的 config 模板里有它们的合法文本。"
        "（规则文案里别写 emoji：Windows 控制台是 GBK，印不出来会把违规报告吞掉）",
    ),
    # ---- 前端边界（见 webui/src/AGENTS.md 与 webui/src/api/AGENTS.md）----
    (
        "components 不许 import views",
        ["webui/src/components"],
        r"from ['\"]\.\./views/|from ['\"]@/views/",
        "方向单向：views 用 components；反过来会成环，也让组件无法独立复用",
    ),
    (
        "作用域查询串只在 api 层拼",
        [
            "webui/src/views",
            "webui/src/components",
            "webui/src/stores",
            "webui/src/composables",
            # ⚠️ router.js 以前不在名单里，而它正是漏得最久的一处（入口守卫自己拼 status 查询串）。
            # 单文件路径也走 source_files()。
            "webui/src/router.js",
        ],
        r"[?&]pid=|\bscopeQ\(|\bscopeQuery\(",
        "`api/scope.js::scopeQ` 是唯一入口（`api/projectData.js::paths` 给它包了一层）；散着拼会漏 scope，"
        "越权/串项目就出在这里。旧判据只认 `[?&]pid=`，而实际代码写的是 `?${scopeQ(pid)}` —— "
        "正则看不见，于是这条不变式长期只活在散文里",
    ),
]

# 成对判据，两种形态：
#   * **`need` 是正则**：命中 `trigger` 的文件必须同时出现 `need`（"用了 X 就必须有 Y"）；
#   * **`need` 是 `None`**：命中 `trigger` 就是违规（"这个文件不许出现这些词"）。
#
# 为什么单独立一类：`RULES` 是"某行不许出现 X"的行级正则，而"用了 Agent 就必须交出取消探针"
# 是文件级约束 —— 正确写法里 `Agent::new(` 与 `with_cancel(` 本来就在不同行（有时还隔着
# `match`），行级正则表达不出来。判据刻意做粗（文件级），因为它的目的是"别漏掉新的批量循环"，
# 不是精确到语句；宁可偶尔要求一次人工确认，也不要脆到一改格式就失效。
PAIRED_RULES: list[tuple[str, list[str], str, str | None, str]] = [
    (
        "把取消探针交给 Agent",
        ["crates/yekitrim-polish/src/llm", "crates/yekitrim-core/src/asr/correct.rs"],
        r"Agent::new\(",
        r"with_cancel\(",
        "批量 LLM 循环必须在**窗口边界**响应取消（`Agent::with_cancel`）：只在阶段边界查一次 = "
        "用户点了停止、后台仍把窗口一个个（甚至成批）跑完 —— 检测链路实测 18 个窗口、十几分钟",
    ),
    # ---- 领域词隔离（整改 T17）----
    # 为什么要有这一类：门禁原本只防「import 汇合」（两个 worker 不许互相 import），于是
    # 3,235 行的单文件成了**合法绕过通道** —— 录播姬的待分配池语义与 B站的提交即下载语义
    # 住在同一个文件里，两个入口相隔 2000 行。**语义汇合得用语义判据拦**，而代价最低的抓手
    # 就是对方的领域词：真把两类逻辑混进来，这些词几乎必然出现在代码里。
    # ⚠️ 匹配前已剥注释（`strip_comments`），所以文档里写"别写 bvid"是安全的。
    (
        "webhook 侧不许出现 B站领域词",
        # `webhook.rs` 只是入口；解析（`webhook_event`）与路径/产物（`webhook_path`）同样是
        # **录播姬那一侧**，一起受约束 —— 只钉一个文件的话，混进来的语义换个文件名就能躲过去。
        # ⚠️ `mod.rs` 刻意不在名单里：它的 `pub use bilibili::{…}` / `pub use webhook::{…}`
        # 必须同时提到两侧（那是这个目录的对外面，不是业务逻辑）。
        [
            "crates/yekitrim-server/src/ingest/webhook.rs",
            "crates/yekitrim-server/src/ingest/webhook_event.rs",
            "crates/yekitrim-server/src/ingest/webhook_path.rs",
        ],
        r"bilibili|bvid|yt-dlp|yt_dlp",
        None,
        "录播姬 webhook 是「无身份 → 待分配池 → 靠空闲窗口完结」，B站是「用户主动提交 → 有身份 → "
        "进程退出即完结」：两种语义混在一个文件里就会互相踩（当年就是这么绕过 worker 那条禁令的）。"
        "要写 B站的东西放 ingest/bilibili.rs",
    ),
    (
        "bilibili 侧不许出现录播姬领域词",
        ["crates/yekitrim-server/src/ingest/bilibili.rs"],
        r"\brecorder\b|\broom_id\b|待分配",
        None,
        "上一条的反向：B站链路（resolve / submit）不许长出录播姬的会话 / 房间语义 —— "
        "ingest_sessions 是两条链共用的，靠 kind 区分，别拿 room_id 塞 bvid",
    ),
]


def say(text: str) -> None:
    """打印一行 —— **编不出来也要打得出来**。

    ⚠️ Windows 控制台/管道默认是 GBK：规则文案里的 ⚠️ 这类字符会让 print 直接抛
    UnicodeEncodeError，而抛出点正是"打印违规"那一行 —— 表现是**门禁崩了、违规清单没出来**
    （实测：变异自证只看到 traceback，看不到是哪一条违规）。宁可退化成 "?"，也不能吞报告。
    """
    try:
        print(text)
    except UnicodeEncodeError:
        enc = sys.stdout.encoding or "utf-8"
        print(text.encode(enc, "replace").decode(enc, "replace"))


def strip_comments(line: str) -> str:
    """去掉 `//` 起的注释，但保留 `https://` 这类 URL。

    宁可少剥（假阴性）也不多剥（假阳性）：多剥会把真违规藏起来，这比报出噪音严重。
    """
    return re.sub(r"(?<!:)//.*$", "", line)


# 判据要同时覆盖 Rust 与前端（`components → views`、查询串拼装这两条是前端的）。
SOURCE_EXTENSIONS = (".rs", ".vue", ".js", ".mjs", ".ts", ".css")


def source_files(rel: str) -> list[Path]:
    p = REPO / rel
    if p.is_file():
        return [p]
    if not p.is_dir():
        return []
    return sorted(
        f for f in p.rglob("*") if f.is_file() and f.suffix in SOURCE_EXTENSIONS
    )


# 规则例外：按 `规则名 -> 允许命中的路径子串` 走。
# ⚠️ 只给"规则定义者自己"开口子（例如那条不变式的**实现处**），别拿来放业务代码。
EXEMPT: dict[str, list[str]] = {
    "子进程不许绕过 core 的启动助手": [
        # ① 这条规则的实现本身就是"唯一允许直接 Command::new 的地方"
        "crates/yekitrim-core/src/process.rs",
        # ② 托盘是**薄启动器**：它刻意不链接任何 `yekitrim-*` crate —— 只要引用 core 的
        #    任何一个符号，链接器就会把静态链接的 ASR 引擎（sherpa-onnx）一起拉回来，
        #    托盘从 0.49 MB 涨回 57 MB。所以它**用不了** `process::cmd()`，改为自己设
        #    `CREATE_NO_WINDOW`（tray.rs 里两处：起 server / taskkill）。这是**刻意换体积**的取舍。
        "crates/yekitrim-cli/src/tray.rs",
    ],
    # ⚠️ 这条的例外就是**规则名字自己点名的那个 crate**：wire 字面量的唯一合法住处。
    # 它不是"业务代码开了个口子"，而是这条不变式的**另一面**（与上面 process.rs 同类）。
    # 别往里加第二个目录 —— 那等于把这条规则作废。
    "only_llm_knows_wire": ["crates/yekitrim-llm/"],
}


# ── 卫生判据 ───────────────────────────────────────────────────────────────
# 为什么单独立一类：上面两类都是"某些行不许出现 X"，这一条是**整棵工作区**的属性。
HYGIENE_RULES = ("工作副本不许有 CRLF",)


def check_line_endings() -> list[str]:
    """`git ls-files --eol` 里 **工作副本** 出现 `w/crlf` 即违规。

    `.gitattributes` 承诺 `* text=auto eol=lf`（源码统一 LF 存储 + LF 检出），但 git 只在
    检出时应用它，**不会重写已经在工作区里的文件** —— 任何会做换行翻译的写入方式
    （Python `write_text` 默认就把 `
` 翻成 `
`）都能在不知不觉中把工作副本变成 CRLF。

    为什么必须机器拦（实测连踩三轮）：**变异自证型契约脚本会静默失效** ——
    它们在源码文本上做 `.replace('…
…')`，CRLF 让锚点匹配不上 → 变异不生效 → 判据不翻面，
    脚本**照旧打印通过**；同时 `webui/dist` 的 chunk hash 整体漂移（dist 与 src 失同步）。
    这类失败没有任何报错，只有"门禁拦下来"能防住。
    """
    try:
        r = subprocess.run(
            ["git", "ls-files", "--eol"], capture_output=True, text=True, encoding="utf-8"
        )
    except OSError:
        return []  # 没有 git：跳过（别把环境问题当成违规）
    if r.returncode != 0 or not r.stdout.strip():
        return []
    bad: list[str] = []
    for line in r.stdout.splitlines():
        # 形如：`i/lf    w/crlf  attr/text=auto eol=lf	<path>`
        # ⚠️ `w/mixed` 也要算 —— 只差一行 CR 同样是"这个文件不是纯 LF"，
        #     而契约脚本按 `\n` 找锚点，一行 CR 就足以让某个锚点匹配不上（本脚本自己就中过）。
        if "w/crlf" in line or "w/mixed" in line:
            bad.append(line.split("	")[-1])
    return bad


def main() -> int:
    verbose = "-v" in sys.argv
    total = 0
    checked = 0

    for name, paths, pattern, why in RULES:
        rx = re.compile(pattern)
        hits: list[str] = []
        files: list[Path] = []
        for rel in paths:
            files.extend(source_files(rel))

        if not files:
            say(f"[!!] {name}：规则指向的路径不存在（判据已失效，请修脚本）")
            total += 1
            continue
        checked += len(files)

        exempt = EXEMPT.get(name, [])
        for f in files:
            rel_f = f.relative_to(REPO).as_posix()
            if any(sub in rel_f for sub in exempt):
                continue
            for i, line in enumerate(f.read_text(encoding="utf-8", errors="replace").splitlines(), 1):
                code = strip_comments(line)
                if rx.search(code):
                    hits.append(f"{f.relative_to(REPO).as_posix()}:{i}: {code.strip()[:100]}")

        if hits:
            total += len(hits)
            say(f"[!!] {name}  —— {why}")
            for h in hits:
                say(f"       {h}")
        elif verbose:
            say(f"[OK] {name}（{len(files)} 个文件）")

    # 成对判据（文件级）：见 PAIRED_RULES 的说明
    for name, paths, trigger, need, why in PAIRED_RULES:
        tx = re.compile(trigger)
        nx = re.compile(need) if need is not None else None
        files: list[Path] = []
        for rel in paths:
            files.extend(source_files(rel))
        if not files:
            say(f"[!!] {name}：规则指向的路径不存在（判据已失效，请修脚本）")
            total += 1
            continue
        checked += len(files)
        for f in files:
            body = "\n".join(
                strip_comments(line)
                for line in f.read_text(encoding="utf-8", errors="replace").splitlines()
            )
            if nx is None:
                # 禁止形态：出现了就算违规（不看别的文件）
                if tx.search(body):
                    total += 1
                    say(f"[!!] {name}  —— {why}")
                    say(
                        f"       {f.relative_to(REPO).as_posix()}："
                        f"出现了禁止的领域词 `{trigger}`（这一行该挪到对方的文件里）"
                    )
                continue
            if tx.search(body) and not nx.search(body):
                total += 1
                say(f"[!!] {name}  —— {why}")
                say(
                    f"       {f.relative_to(REPO).as_posix()}："
                    f"用了 `{trigger}` 但全文没有 `{need}`"
                )
        if verbose and not any(
            tx.search(
                "\n".join(
                    strip_comments(line)
                    for line in f.read_text(encoding="utf-8", errors="replace").splitlines()
                )
            )
            for f in files
        ):
            print(f"[OK] {name}（{len(files)} 个文件，无一命中）")

    # 卫生判据：整棵工作区的属性（见 check_line_endings 的说明）
    crlf = check_line_endings()
    if crlf:
        total += len(crlf)
        print(f"[!!] {HYGIENE_RULES[0]}  —— 工作副本是 CRLF 的文件 {len(crlf)} 个：")
        for f in crlf[:10]:
            print(f"       {f}")
        if len(crlf) > 10:
            print(f"       …还有 {len(crlf) - 10} 个")
        print("       修法：把这些文件转回 LF（写入时别让工具做换行翻译），再重跑。")
    elif verbose:
        print(f"[OK] {HYGIENE_RULES[0]}（工作副本全是 LF）")

    print()
    if total:
        print(f">>> {total} 处违规，边界被破坏。别绕过这个检查 —— 破了要说明理由并同步改规则。")
        return 1
    print(
        f">>> 边界完整：{len(RULES) + len(PAIRED_RULES) + len(HYGIENE_RULES)} 条规则"
        f" / {checked} 个文件，0 违规。"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
