#!/usr/bin/env python3
"""出片日志的**阶段报告**（只读）：把 `render-*.log` 拆成
「墙钟 / 分块渲染 / 抽帧 / 编码 / 固定开销」与「发帧+启动 / 页面循环 / 回读」两张表。

配套文档：`docs/plan/polish-render-speed.md`（§1 的两张表就是这么来的）。

用法::

    python scripts/render_phase_report.py [日志目录]
    # 默认目录：<仓库>/target/debug/.vtrim/cloud/logs

为什么需要它：出片日志里只有分块级的「抽帧 Xs / 渲染 Ys / 编码 Zs」，
而**「渲染」那一段把浏览器里的逐帧耗时和两端的 base64/JSON 搬运混在一起**
（见文档 §1.2）。这里用日志时间戳把它切成三段 —— 不用改代码、不用重跑，就能看出
「钱花在搬画面还是画画面」：

    本块渲染段起点 = `Encoder:` 行时刻 + 本块抽帧耗时
    渲染段终点     = `chunk#N: done` 行时刻 − 本块编码耗时
    ① 发帧+启动 = 第一次 `frames A..B: N% Ns` 进度行 − 渲染段起点
    ② 页面循环 = 末次进度行 − 首次进度行
    ③ 回读     = 渲染段终点 − 末次进度行
    校验：①+②+③ 必须 ≈ 日志里的「渲染」耗时（误差 ~0.1s，脚本会打出来）

⚠️ 页面的进度回写每 30 帧/1.5s 一次，所以 ①②③ 的切点是"最近的一次回写"，
不是精确边界（页面循环那一段误差 ≤ 1.5s，摊到每帧 < 0.1ms）。

零依赖、只读；日志是 GBK/UTF-8 混合（Windows 中文），解码不上就退 GBK。
"""

from __future__ import annotations

import datetime
import pathlib
import re
import sys

TS = re.compile(r"^(\d{4}-\d{2}-\d{2}T\d{2}:\d{2}:\d{2}(?:\.\d+)?)Z")
PROGRESS = re.compile(r"frames (\d+)\.\.(\d+): (\d+)%\s+(\d+)s")
CHUNK_DONE = re.compile(
    r"chunk#(\d+): done\s+(\d+) frames[^\d]*([\d.]+)s\s*/\s*[^\d]*([\d.]+)s\s*/\s*[^\d]*([\d.]+)s"
)
ENCODER = re.compile(r"Encoder: (\S+)")
DURATION = re.compile(r"Duration: ([\d.]+)s")
FRAMES = re.compile(r"(\d+) frames at (\d+) fps")
CANVAS = re.compile(r"\u753b\u5e03 (\d+)x(\d+) \u00b7 \u6e90 (\S+) \u2192 (.*)")


def decode(raw: bytes) -> str:
    try:
        return raw.decode("utf-8")
    except UnicodeDecodeError:
        return raw.decode("gbk", errors="replace")


def parse(path: pathlib.Path) -> dict | None:
    lines = decode(path.read_bytes()).splitlines()

    def first(rx):
        for ln in lines:
            m = rx.search(ln)
            if m:
                return m
        return None

    duration, frames, encoder, canvas = (
        first(DURATION),
        first(FRAMES),
        first(ENCODER),
        first(CANVAS),
    )
    t_enc = None
    first_prog: dict[tuple[int, int], tuple] = {}
    last_prog: dict[tuple[int, int], tuple] = {}
    done: dict[int, tuple] = {}
    stamps: list[datetime.datetime] = []
    for ln in lines:
        m = TS.match(ln)
        if not m:
            continue
        t = datetime.datetime.fromisoformat(m.group(1))
        stamps.append(t)
        if encoder and encoder.group(0) in ln and t_enc is None:
            t_enc = t
        if (mp := PROGRESS.search(ln)) is not None:
            key = (int(mp.group(1)), int(mp.group(2)))
            rec = (t, int(mp.group(3)), int(mp.group(4)))
            first_prog.setdefault(key, rec)
            last_prog[key] = rec
        if (md := CHUNK_DONE.search(ln)) is not None:
            done[int(md.group(1))] = (
                t,
                int(md.group(2)),
                float(md.group(3)),
                float(md.group(4)),
                float(md.group(5)),
            )
    if not (duration and frames and t_enc and done and stamps):
        return None

    chunks = []
    for idx in sorted(done):
        t_done, nf, ex, rd, en = done[idx]
        keys = [k for k in first_prog if abs((k[1] - k[0]) - nf) <= 1]
        if not keys:
            continue
        key = min(keys, key=lambda k: abs((k[1] - k[0]) - nf))
        t_first, _, _ = first_prog[key]
        t_last, _, _ = last_prog[key]
        render_start = t_enc + datetime.timedelta(seconds=ex)
        send = (t_first - render_start).total_seconds()
        page = (t_last - t_first).total_seconds()
        read = (t_done - datetime.timedelta(seconds=en) - t_last).total_seconds()
        chunks.append(
            dict(
                idx=idx, nf=nf, extract=ex, render=rd, encode=en,
                send=send, page=page, read=read,
                check=send + page + read - rd,
            )
        )

    wall = (max(stamps) - min(stamps)).total_seconds()
    return dict(
        name=path.name,
        duration=float(duration.group(1)),
        frames=int(frames.group(1)),
        encoder=encoder.group(1),
        canvas=canvas.group(0) if canvas else "",
        wall=wall,
        chunks=chunks,
    )


def main() -> int:
    root = pathlib.Path(sys.argv[1]) if len(sys.argv) > 1 else (
        pathlib.Path(__file__).resolve().parent.parent
        / "target" / "debug" / ".vtrim" / "cloud" / "logs"
    )
    if not root.is_dir():
        print(f"没有这个目录: {root}")
        return 1

    reps = [r for p in sorted(root.glob("render-*.log"), key=lambda x: x.stat().st_mtime)
            if (r := parse(p)) and r["chunks"]]
    if not reps:
        print(f"{root} 里没有可解析的 render 日志")
        return 1

    print("① 每次出片：墙钟 / 并行段 / 固定开销 / 有效帧率")
    print(f"{'日志':24} {'片段s':>7} {'帧数':>6} {'并行':>4} {'墙钟s':>7} {'并行段s':>8} "
          f"{'固定s':>6} {'渲染Σs':>7} {'抽帧Σs':>7} {'编码Σs':>6} {'聚合fps':>7} {'实时':>6}  画布/源")
    for r in reps:
        par = len(r["chunks"])
        render_sum = sum(c["render"] for c in r["chunks"])
        extract_sum = sum(c["extract"] for c in r["chunks"])
        encode_sum = sum(c["encode"] for c in r["chunks"])
        # 并行段 = 最慢那一块的总耗时（抽帧+渲染+编码 串行）
        par_wall = max(c["extract"] + c["render"] + c["encode"] for c in r["chunks"])
        fixed = r["wall"] - par_wall
        print(f"{r['name']:24} {r['duration']:7.1f} {r['frames']:6d} {par:4d} {r['wall']:7.1f} "
              f"{par_wall:8.1f} {fixed:6.1f} {render_sum:7.1f} {extract_sum:7.1f} {encode_sum:6.1f} "
              f"{r['frames'] / r['wall']:7.1f} {r['duration'] / r['wall']:5.2f}x  {r['canvas']}")

    print("\n② 每块渲染段的成分：发帧+启动 / 页面循环 / 回读（ms/帧）")
    print(f"{'日志':24} {'块':>3} {'帧数':>6} {'渲染段':>7} {'发帧':>7} {'页面':>7} {'回读':>6} {'校验s':>7}")
    for r in reps:
        for c in r["chunks"]:
            n = c["nf"] or 1
            print(f"{r['name']:24} {c['idx']:3d} {c['nf']:6d} {c['render'] / n * 1000:7.1f} "
                  f"{c['send'] / n * 1000:7.1f} {c['page'] / n * 1000:7.1f} {c['read'] / n * 1000:6.1f} "
                  f"{c['check']:7.2f}")

    total_send = sum(c["send"] for r in reps for c in r["chunks"])
    total_page = sum(c["page"] for r in reps for c in r["chunks"])
    total_read = sum(c["read"] for r in reps for c in r["chunks"])
    total = total_send + total_page + total_read
    if total > 0:
        print(f"\n合计占比：发帧+启动 {total_send / total:.0%} · 页面循环 {total_page / total:.0%} "
              f"· 回读 {total_read / total:.0%}（全部 {len(reps)} 次出片、"
              f"{sum(r['frames'] for r in reps)} 帧）")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
