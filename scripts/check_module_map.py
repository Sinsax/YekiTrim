#!/usr/bin/env python3
"""模块卡门禁：把"按模块分离"变成可执行的判断，而不是靠自觉。

为什么要有这个脚本
------------------
规则从根 `AGENTS.md` 拆进了各模块目录的 `AGENTS.md`（DSH 会在 agent 的
read/write/edit **触到该目录**时自动注入那一份）。好处是"到点看到规则"，
代价是**卡会漂移**：新目录没人开卡、卡写得太大把预算挤爆、规则被顺手删掉、
注册表与索引表对不上。这些都不会报错、测试也不会红，所以在这里逐条钉住。

判据为什么长这样
----------------
1. **预算按"最坏链"算，不是按单文件**：DSH 一次注入的是"根 + 沿路径每一级卡"，
   总预算 65536 B。超预算时 DSH **先整份丢掉较宽的文件**——根 `AGENTS.md` 一旦超标，
   全局规矩会全部消失（这正是一次要拆的原因）。所以既卡单文件，也卡链。
2. **覆盖率排除 root 模块**：`root` 的 `dir` 是 `.`，它"覆盖一切"。若把它算作所有者，
   任何没开卡的目录都会被它兜住、覆盖率形同虚设。所以只有 `kind != "root"` 的模块算所有者。
3. **⚠️ 下限**：迁移前的根 AGENTS.md 有 117 行含 ⚠️ 的规则（踩坑换来的）。
   拆卡时只许多、不许少——这条防的是"清理文档时顺手抹掉警告"。
4. **索引表用显式标记**：根卡片里只有 `<!-- module-index:begin -->` 与 `<!-- module-index:end -->`
   之间的表格参与校验，免得把状态机表、防线表误当模块行。

用法
----
    python3 scripts/check_module_map.py        # 违规非零退出
    python3 scripts/check_module_map.py -v     # 同时列出通过项
    python3 scripts/check_module_map.py --json # 机器可读报告

⚠️ 用 `python3`：本机（以及不少 Windows 环境）`python` 不在 PATH。
"""

from __future__ import annotations

import argparse
import glob
import json
import os
import sys
from pathlib import Path

REPO = Path(__file__).resolve().parent.parent
REGISTRY = REPO / "docs" / "module-map.json"

INDEX_BEGIN = "<!-- module-index:begin -->"
INDEX_END = "<!-- module-index:end -->"

REQUIRED_SECTIONS = ["## 职责", "## 入口", "## 边界", "## 验证"]

# 每张卡都必须写明的"别越界"协议：这是"防止 agent 连带修改"的落点 ——
# 卡会在 agent 触到该目录时被自动注入，所以这句话**必须**出现在每张卡里，而不是只在根文件里。
BOUNDARY_PROTOCOL = "改动只允许落在本目录"


def rel(path: Path) -> str:
    return path.relative_to(REPO).as_posix()


def size(path: Path) -> int:
    return path.stat().st_size  # 磁盘字节数：预算判据要与 DSH 看到的字节一致


def is_under(child: str, parent: str) -> bool:
    """`child` 是否在 `parent` 目录内（含相等）。两者都是仓库相对 posix 路径。"""
    if parent == ".":
        return True
    return child == parent or child.startswith(parent + "/")


class Report:
    def __init__(self) -> None:
        self.problems: list[str] = []
        self.passed: list[str] = []
        self.data: dict[str, object] = {}

    def fail(self, message: str) -> None:
        self.problems.append(message)

    def ok(self, message: str) -> None:
        self.passed.append(message)


def load_registry(rep: Report) -> dict | None:
    if not REGISTRY.is_file():
        rep.fail(f"找不到注册表 {rel(REGISTRY)}（模块结构没有单一来源，门禁失效）")
        return None
    try:
        data = json.loads(REGISTRY.read_text(encoding="utf-8"))
    except json.JSONDecodeError as err:
        rep.fail(f"{rel(REGISTRY)} 不是合法 JSON：{err}")
        return None
    for key in ("version", "budgets", "warning_floor", "coverage_roots", "modules"):
        if key not in data:
            rep.fail(f"{rel(REGISTRY)} 缺字段 `{key}`")
            return None
    if data["version"] != 1:
        rep.fail(f"{rel(REGISTRY)} 的 version={data['version']}，本脚本只认 1")
        return None
    return data


def check_registry_shape(data: dict, rep: Report) -> None:
    modules = data["modules"]
    ids = [m.get("id") for m in modules]
    dirs = [m.get("dir") for m in modules]
    dup_ids = sorted({i for i in ids if i is not None and ids.count(i) > 1})
    dup_dirs = sorted({d for d in dirs if d is not None and dirs.count(d) > 1})
    if dup_ids:
        rep.fail(f"注册表里有重复 id：{dup_ids}")
    if dup_dirs:
        rep.fail(f"注册表里有重复 dir：{dup_dirs}")

    for m in modules:
        mid = m.get("id", "?")
        for key in ("id", "title", "dir", "card", "kind", "verify"):
            if key not in m:
                rep.fail(f"模块 `{mid}` 缺字段 `{key}`")
        d = m.get("dir")
        c = m.get("card")
        if not d or not c:
            continue
        want = "AGENTS.md" if d == "." else f"{d}/AGENTS.md"
        if c != want:
            rep.fail(f"模块 `{mid}` 的 card 必须是 `{want}`（实际 `{c}`）—— 卡只认这个位置与文件名")
        if not (REPO / d).is_dir():
            rep.fail(f"模块 `{mid}` 的 dir 不存在：`{d}`")
        if not (REPO / c).is_file():
            rep.fail(f"模块 `{mid}` 的卡不存在：`{c}`")
        if not isinstance(m.get("verify"), list) or not m.get("verify"):
            rep.fail(f"模块 `{mid}` 的 verify 必须是非空数组")
    rep.ok(f"注册表结构：{len(modules)} 个模块，id/dir 唯一，dir 与卡都在磁盘上")


def check_card_skeleton(data: dict, rep: Report) -> None:
    for m in data["modules"]:
        card = REPO / m["card"]
        if not card.is_file():
            continue  # 上一个检查已经报过
        text = card.read_text(encoding="utf-8", errors="replace")
        lines = [line.rstrip() for line in text.splitlines()]

        missing = [s for s in REQUIRED_SECTIONS if not any(l.startswith(s) for l in lines)]
        if missing:
            rep.fail(f"`{m['card']}` 缺小节：{'、'.join(missing)}（四节骨架不许改名/删减）")

        scope_lines = [l for l in lines if l.startswith("> 范围：")]
        if not scope_lines:
            rep.fail(f"`{m['card']}` 缺 `> 范围：` 元信息行")
        elif m["dir"] not in scope_lines[0]:
            rep.fail(f"`{m['card']}` 的 `> 范围：` 行必须包含目录 `{m['dir']}`（实际：{scope_lines[0][:80]}）")

        for command in m["verify"]:
            if command not in text:
                rep.fail(f"`{m['card']}` 的 `## 验证` 里找不到注册表的 verify 命令：`{command}`")

        if m["kind"] != "root" and BOUNDARY_PROTOCOL not in text:
            rep.fail(
                f"`{m['card']}` 缺越界协议行「{BOUNDARY_PROTOCOL}」"
                "（卡被注入时必须告诉 agent：越界改动要先读对方卡并在提交说明里写理由）"
            )
    rep.ok("卡骨架：四节小节、`> 范围：` 行、越界协议、verify 命令齐全")


def check_budgets(data: dict, rep: Report) -> None:
    budgets = data["budgets"]
    root_bytes = budgets["root_bytes"]
    module_bytes = budgets["module_bytes"]
    chain_bytes = budgets["chain_bytes"]

    sizes: dict[str, int] = {}
    for m in data["modules"]:
        card = REPO / m["card"]
        if card.is_file():
            sizes[m["id"]] = size(card)

    root = next((m for m in data["modules"] if m["kind"] == "root"), None)
    if root is None:
        rep.fail("注册表里必须有且只有一个 kind=root 的模块")
        return
    root_size = sizes.get("root", 0)
    if root_size > root_bytes:
        rep.fail(
            f"根 `AGENTS.md` 超预算：{root_size} B > {root_bytes} B。"
            "根文件超标时 DSH 会整份丢掉它（连模块索引一起没），必须继续拆到模块卡"
        )

    for m in data["modules"]:
        if m["kind"] == "root":
            continue
        got = sizes.get(m["id"], 0)
        if got > module_bytes:
            rep.fail(f"模块卡 `{m['card']}` 超预算：{got} B > {module_bytes} B（把冗长叙述挪去 docs/ 指针）")

    # 最坏链：根 + 沿路径每一级祖先卡
    worst = ("", 0)
    for m in data["modules"]:
        if m["kind"] == "root":
            continue
        chain = root_size
        names = ["root"]
        for other in data["modules"]:
            if other["kind"] == "root":
                continue
            if is_under(m["dir"], other["dir"]):
                chain += sizes.get(other["id"], 0)
                names.append(other["id"])
        if chain > worst[1]:
            worst = (" + ".join(names), chain)
        if chain > chain_bytes:
            rep.fail(
                f"最坏指令链超预算（{chain} B > {chain_bytes} B）：{' + '.join(names)}。"
                "DSH 一次注入的正是这条链，超了就会丢文件"
            )
    rep.ok(
        f"预算：根 {root_size}/{root_bytes} B，单卡 ≤{module_bytes} B，"
        f"最坏链 {worst[1]}/{chain_bytes} B（{worst[0]}）"
    )
    rep.data["root_bytes"] = root_size
    rep.data["worst_chain"] = {"modules": worst[0], "bytes": worst[1]}


def check_coverage(data: dict, rep: Report) -> None:
    owners = [m for m in data["modules"] if m["kind"] != "root"]
    exclude = tuple(data.get("exclude", []))

    def excluded(relpath: str) -> bool:
        return any(relpath == e or relpath.startswith(e + "/") for e in exclude)

    checked = 0
    unowned: list[str] = []
    empty_patterns: list[str] = []
    for pattern in data["coverage_roots"]:
        matched = [p for p in glob.glob(str(REPO / pattern)) if os.path.isdir(p)]
        if not matched:
            empty_patterns.append(pattern)
            continue
        for base in matched:
            for dirpath, dirnames, filenames in os.walk(base):
                dirnames[:] = [d for d in dirnames if not excluded(f"{rel(Path(dirpath))}/{d}")]
                for name in filenames:
                    path = Path(dirpath) / name
                    r = rel(path)
                    if excluded(r):
                        continue
                    checked += 1
                    if not any(is_under(r, m["dir"]) for m in owners):
                        unowned.append(r)

    if empty_patterns:
        rep.fail(
            "coverage_roots 里有模式匹配不到任何目录（多半是路径写错了）："
            + "、".join(f"`{p}`" for p in empty_patterns)
        )
    if unowned:
        head = "、".join(f"`{u}`" for u in sorted(unowned)[:8])
        more = "" if len(unowned) <= 8 else f"（共 {len(unowned)} 个）"
        rep.fail(f"有 {len(unowned)} 个源文件不属于任何模块卡：{head}{more} —— 给它们开一张卡，别让 agent 走进无主区")
    else:
        rep.ok(f"覆盖率：{checked} 个源文件全部落在某个模块目录内")


def check_reverse_coverage(data: dict, rep: Report) -> None:
    cards = {m["card"] for m in data["modules"]}
    exclude = tuple(data.get("exclude", []))

    def excluded(relpath: str) -> bool:
        return any(relpath == e or relpath.startswith(e + "/") for e in exclude)

    found: list[str] = []
    for dirpath, dirnames, filenames in os.walk(REPO):
        dirnames[:] = [d for d in dirnames if not excluded(f"{rel(Path(dirpath))}/{d}")]
        if "AGENTS.md" in filenames:
            found.append(rel(Path(dirpath) / "AGENTS.md"))
    stray = sorted(set(found) - cards)
    if stray:
        rep.fail(
            "仓库里有没登记进注册表的 AGENTS.md（卡片会漂移）："
            + "、".join(f"`{s}`" for s in stray)
        )
    missing = sorted(c for c in cards if not (REPO / c).is_file())
    if missing:
        rep.fail("注册表登记了卡但文件不存在：" + "、".join(f"`{m}`" for m in missing))
    rep.ok(f"反向覆盖：{len(found)} 个 AGENTS.md 与注册表一一对应")


def check_module_index(data: dict, rep: Report) -> None:
    root = next((m for m in data["modules"] if m["kind"] == "root"), None)
    if root is None:
        return
    card = REPO / root["card"]
    if not card.is_file():
        return
    text = card.read_text(encoding="utf-8", errors="replace")
    if INDEX_BEGIN not in text or INDEX_END not in text:
        rep.fail(f"根 `{root['card']}` 缺模块索引表标记 `{INDEX_BEGIN}` / `{INDEX_END}`")
        return
    body = text.split(INDEX_BEGIN, 1)[1].split(INDEX_END, 1)[0]
    listed: list[str] = []
    for line in body.splitlines():
        line = line.strip()
        if not line.startswith("|"):
            continue
        first = line.strip("|").split("|")[0].strip().strip("`").strip()
        if not first or set(first) <= set("-: "):
            continue
        if first in ("id", "模块"):
            continue
        listed.append(first)

    want = {m["id"] for m in data["modules"]}
    got = set(listed)
    if got != want:
        missing = sorted(want - got)
        extra = sorted(got - want)
        detail = []
        if missing:
            detail.append("索引表少了：" + "、".join(f"`{i}`" for i in missing))
        if extra:
            detail.append("索引表多了（注册表里没有）：" + "、".join(f"`{i}`" for i in extra))
        rep.fail(f"根 `{root['card']}` 的模块索引表与注册表不一致 —— " + "；".join(detail))
    else:
        rep.ok(f"模块索引表：{len(want)} 条与注册表一致")


def count_warning_lines(data: dict) -> int:
    total = 0
    for m in data["modules"]:
        card = REPO / m["card"]
        if not card.is_file():
            continue
        for line in card.read_text(encoding="utf-8", errors="replace").splitlines():
            if "\u26a0" in line:  # ⚠（含 ⚠️）/ 变体
                total += 1
    return total


def check_warning_floor(data: dict, rep: Report) -> None:
    floor = data["warning_floor"]
    got = count_warning_lines(data)
    if got < floor:
        rep.fail(
            f"⚠️ 规则行只剩 {got} 行，低于下限 {floor} —— 拆卡时丢了踩坑记录。"
            "要么补回原文，要么在注册表里写明理由后下调 warning_floor"
        )
    else:
        rep.ok(f"⚠️ 下限：{got} 行 ≥ {floor}")
    rep.data["warning_lines"] = got


def main() -> int:
    parser = argparse.ArgumentParser(description="YekiTrim 模块卡门禁")
    parser.add_argument("-v", "--verbose", action="store_true", help="同时列出通过项")
    parser.add_argument("--json", action="store_true", help="输出机器可读报告")
    args = parser.parse_args()

    rep = Report()
    data = load_registry(rep)
    if data is not None:
        check_registry_shape(data, rep)
        check_card_skeleton(data, rep)
        check_budgets(data, rep)
        check_coverage(data, rep)
        check_reverse_coverage(data, rep)
        check_module_index(data, rep)
        check_warning_floor(data, rep)

    if args.json:
        print(json.dumps(
            {"problems": rep.problems, "passed": rep.passed, "data": rep.data},
            ensure_ascii=False, indent=2,
        ))
        return 1 if rep.problems else 0

    if args.verbose:
        for line in rep.passed:
            print(f"[OK] {line}")
    if rep.problems:
        for line in rep.problems:
            print(f"[!!] {line}")
        print()
        print(f">>> {len(rep.problems)} 处模块结构问题。修完再跑一遍；别绕过这个检查。")
        return 1
    print(f">>> 模块结构完整：{len(data['modules'])} 个模块 / 7 项检查全过。")
    return 0


if __name__ == "__main__":
    sys.exit(main())
