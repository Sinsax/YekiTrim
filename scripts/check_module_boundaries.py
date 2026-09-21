#!/usr/bin/env python3
r"""模块边界门禁：把"是否嵌合"变成可执行的判断，而不是靠感觉。

为什么要有这个脚本
------------------
两类**硬边界**写成散文就会腐坏 —— 一开始没人违反，后来某次"就这一次"加个 import
就破防了，而且不会报错、测试也不会红。所以这里把它变成一个本地/CI 都能跑的检查：

1. **P4（B站链接通道）的隔离**（见 `docs/plan/p4-implementation-plan.md` §2）：
   谁不许认识谁、谁不许开子进程。
2. **分层方向与前端边界**（见根 `AGENTS.md`「架构」与各模块卡的「边界」小节）：
   `core` / `polish` / `render-core` 不许认识上层；`vtrim-service` 不许 `use vtrim_server`
   （这条以前只写在 `vtrim-service/src/lib.rs` 的模块注释里，没有任何机器判据）；
   `vtrim-server` 不许碰 `vtrim-cli`；CLI 只经 `vtrim_server::{run, ServerArgs}` 进服务端；
   前端 `components/` 不许 import `views/`；作用域查询串只许在 `webui/src/api/` 里拼。

判据为什么长这样（几条豁免，都是踩过的坑）
------------------------------------------
1. **先剥注释再匹配**：规则说明自己就会写下"本目录不许出现 `yt-dlp`"，不剥注释的话
   这些文档行会全部命中 —— 上一版手写的 grep 判据就因此产出 8 条假阳性，等于没用。
   `(?<!:)//` 是为了不把 `https://…` 里的 `//` 当注释起点。
2. **`std::process::id()` 不算"开子进程"**：它只取 PID 给临时目录命名
   （`downloader/ytdlp.rs`、`bilibili/danmaku.rs` 都用了），没有任何进程被创建。
   所以规则认的是 `Command` / `.spawn(`，不是笼统的 `std::process`。
3. **分层规则用 `(?<!["\w])` 而不是裸 crate 名**：`vtrim-service/src/logbus.rs` 有一段测试
   专门断言"三个 crate 前缀都要剥掉"，里面写着字符串字面量 `"vtrim_server::"` ——
   那是数据不是依赖，裸匹配会误报。所以要求 crate 名前面既不是引号也不是标识符字符。
4. **CLI 那条只放行 `run` / `ServerArgs`**：`vtrim-cli` 本来就依赖 `vtrim-core` /
   `vtrim-polish`（入口装配要用），但**服务端能力**只能经 `vtrim_server::run()` /
   `ServerArgs` 进，不许直接调 server 的内部模块。
5. **前端刻意没加的两条**（现状不满足，加了就是红门禁，别盲加）：
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
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent

# (规则名, 相对路径列表, 正则, 为什么)
RULES: list[tuple[str, list[str], str, str]] = [
    (
        "downloader 不许开子进程",
        ["crates/vtrim-service/src/downloader"],
        r"process::Command|\bCommand::new\b|\.spawn\(",
        "执行只在 vtrim-server/download_worker.rs；适配层要能离线可测",
    ),
    (
        "downloader 不许知道 B站",
        ["crates/vtrim-service/src/downloader"],
        r"\bbvid\b|\bcid\b|bilibili\.com|comment\.bilibili",
        "引擎是通用端口；沾上 B站就只能适配一家，加第二个引擎要改这个目录",
    ),
    (
        "bilibili 不许知道下载器",
        ["crates/vtrim-service/src/bilibili"],
        r"yt-?dlp|VTRIM_PROG|VTRIM_PATH|VTRIM_META|process::Command|\.spawn\(",
        "弹幕走旁路（自己 GET），与引擎完全无关；混在一起就没法单测",
    ),
    (
        "net/ 不许知道 B站",
        ["crates/vtrim-service/src/net"],
        r"\bbvid\b|\bcid\b|bilibili\.com",
        "deflate 解码是通用工具，不该知道调用方是谁",
    ),
    (
        "download_worker 不许引用 ingest_worker",
        ["crates/vtrim-server/src/download_worker.rs"],
        r"\bingest_worker\b",
        "两个 worker 并列；共享部分走 ingest::{capacity,landing} 与 pipeline_boot",
    ),
    (
        "download_worker 不许有录播姬语义",
        ["crates/vtrim-server/src/download_worker.rs"],
        r"is_finalized|move_file\(",
        "录播姬靠空闲窗口猜完结、从录像目录搬入；B站进程退出码就是答案，直接写进托管目录",
    ),
    # ---- 分层方向（见根 AGENTS.md「架构」；每条都对应一张模块卡的「边界」小节）----
    (
        "core 不许认识上层",
        ["crates/vtrim-core/src"],
        r"vtrim_polish|vtrim_service|vtrim_server|vtrim_cli",
        "分层是单向的 core → polish → service → server → cli；底层一旦反向引用，两层就粘回去",
    ),
    (
        "render-core 不许认识上层",
        ["crates/vtrim-render-core/src"],
        r"vtrim_polish|vtrim_service|vtrim_server|vtrim_cli",
        "绘制原语只被 polish / wasm 用；它不认识业务",
    ),
    (
        "polish 不许认识 service/server/cli",
        ["crates/vtrim-polish/src"],
        r"vtrim_service|vtrim_server|vtrim_cli",
        "polish 只依赖 core + render-core；要领域能力就必须经调用方注入",
    ),
    (
        "service 不许认识 server",
        ["crates/vtrim-service/src"],
        r'(?<!["\w])vtrim_server::',
        "`lib.rs` 的明文不变式（它能被抽出来的全部理由），以前没有机器判据",
    ),
    (
        "server 不许认识 cli",
        ["crates/vtrim-server/src"],
        r'(?<!["\w])vtrim_cli::',
        "server 只依赖 service；入口在 cli 那一侧",
    ),
    (
        "cli 只用 server 的入口",
        ["crates/vtrim-cli/src"],
        r'(?<!["\w])vtrim_server::(?!(run|ServerArgs))',
        "服务端能力只能经 vtrim_server::run() / ServerArgs 进；不许直接调 server 内部模块",
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
        ["webui/src/views", "webui/src/components", "webui/src/stores", "webui/src/composables"],
        r"[?&]pid=",
        "`api/scope.js::scopeQ` 是唯一入口；散着拼会漏 scope，越权/串项目就出在这里",
    ),
]

# 成对判据：**命中 `trigger` 的文件必须同时出现 `need`**。
#
# 为什么单独立一类：`RULES` 是"某行不许出现 X"的行级正则，而"用了 Agent 就必须交出取消探针"
# 是文件级约束 —— 正确写法里 `Agent::new(` 与 `with_cancel(` 本来就在不同行（有时还隔着
# `match`），行级正则表达不出来。判据刻意做粗（文件级），因为它的目的是"别漏掉新的批量循环"，
# 不是精确到语句；宁可偶尔要求一次人工确认，也不要脆到一改格式就失效。
PAIRED_RULES: list[tuple[str, list[str], str, str, str]] = [
    (
        "把取消探针交给 Agent",
        ["crates/vtrim-polish/src/llm", "crates/vtrim-core/src/asr/correct.rs"],
        r"Agent::new\(",
        r"with_cancel\(",
        "批量 LLM 循环必须在**窗口边界**响应取消（`Agent::with_cancel`）：只在阶段边界查一次 = "
        "用户点了停止、后台仍把窗口一个个（甚至成批）跑完 —— 检测链路实测 18 个窗口、十几分钟",
    ),
]


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
            print(f"[!!] {name}：规则指向的路径不存在（判据已失效，请修脚本）")
            total += 1
            continue
        checked += len(files)

        for f in files:
            for i, line in enumerate(f.read_text(encoding="utf-8", errors="replace").splitlines(), 1):
                code = strip_comments(line)
                if rx.search(code):
                    hits.append(f"{f.relative_to(REPO).as_posix()}:{i}: {code.strip()[:100]}")

        if hits:
            total += len(hits)
            print(f"[!!] {name}  —— {why}")
            for h in hits:
                print(f"       {h}")
        elif verbose:
            print(f"[OK] {name}（{len(files)} 个文件）")

    # 成对判据（文件级）：见 PAIRED_RULES 的说明
    for name, paths, trigger, need, why in PAIRED_RULES:
        tx = re.compile(trigger)
        nx = re.compile(need)
        files: list[Path] = []
        for rel in paths:
            files.extend(source_files(rel))
        if not files:
            print(f"[!!] {name}：规则指向的路径不存在（判据已失效，请修脚本）")
            total += 1
            continue
        checked += len(files)
        for f in files:
            body = "\n".join(
                strip_comments(line)
                for line in f.read_text(encoding="utf-8", errors="replace").splitlines()
            )
            if tx.search(body) and not nx.search(body):
                total += 1
                print(f"[!!] {name}  —— {why}")
                print(
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

    print()
    if total:
        print(f">>> {total} 处违规，边界被破坏。别绕过这个检查 —— 破了要说明理由并同步改规则。")
        return 1
    print(
        f">>> 边界完整：{len(RULES) + len(PAIRED_RULES)} 条规则 / {checked} 个文件，0 违规。"
    )
    return 0


if __name__ == "__main__":
    sys.exit(main())
