#!/usr/bin/env python3
"""T5 真卡拉OK 的**端到端像素判据**（2026-10-11）。

# 它证明什么

**字级时间戳真的决定了"哪个字什么时候出现在画面上"** —— 不是"整片亮度看起来还行"。
判据是**逐步的**：每个字出现时亮像素会跳一档，**每一跳的时刻必须落在那个字自己的
字级时间戳上（±1 帧）**。夹具把最后一个字（`日`）的**字级时间戳单独拉开到 2.5s**，
所以「它必须在 2.5s 之后才出现、之前一个像素都不许有」是一条**能指名道姓**的判据。

# ⚠️ 三条必须知道的（不然会误判成"功能没生效"）

1. **底座不给字体就整条不出字幕**（`subtitle_font_missing`，明说不猜系统字体 —— 设计如此）。
   ⇒ 必须显式 `--font-file`，否则**每一帧都是 0 像素**，看着像"逐字切分没生效"。
2. **`process` 的输入是 `asr/timeline.json`、不是 `data/char-timeline.json`**
   （后者是它的**产物**）。候选顺序 `<片段>/../../asr/` → `<片段>/../asr/` → `<片段>/asr/`。
   ⇒ 只造后者会走**降级路径**（行为正确，但不切分）。
3. **淡入淡出必须是 0**（开着 `karaoke` 时由转译器写进字幕轨）。
   逐字 cue 只有 ~33ms，走不完 350/180ms 的淡入淡出 ⇒ 每条才到 10%/18% 就被顶掉，
   **有的字根本画不出来**（实测 frame 150 = 0 亮像素）、**最后一个字闪一下就没了**。
   本脚本断言的就是"两者都设 0 之后"的那条干净阶梯。

用法（需要 ffmpeg + Dhampir 二进制 + 一份 CJK 字体）：

    python3 scripts/check-karaoke-pixels.py [--dhampir <exe>] [--font <ttf>] [--keep]

⚠️ 它**不进常规门禁**（要真渲染 40s + 需要外部二进制与字体），与
`check-cover-takumi.mjs --golden` / `portrait_backdrop` 那类"要有真工程才能跑"的判据同档。
"""
import argparse
import json
import os
import re
import shutil
import subprocess
import sys
import tempfile

REPO = os.path.dirname(os.path.dirname(os.path.abspath(__file__)))
FPS = 60
DUR = 3.0
TOTAL_FRAMES = int(DUR * FPS)  # 180

# 夹具：15 个填充字 + 第 16 个字是"日"。
# ⚠️ **填充字必须全同**（都用"一"）—— 这样"第 16 个字出现"那一跳在像素上是**孤立**的，
#    不会被别的字的差异掩盖。代价是前 15 跳的分辨率低（同形字），所以判据主要钉最后一个。
PAD = "\u4e00" * 15
TARGET = "\u65e5"
TEXT = PAD + TARGET
# 前 15 个均匀铺在 0..0.5s；**第 16 个单独拉开到 2.5s**。
TIMES = [round(i * 0.5 / 15, 4) for i in range(15)] + [2.5]


def run(cmd, **kw):
    return subprocess.run(cmd, capture_output=True, text=True, encoding="utf-8",
                          errors="replace", **kw)


def build_fixture(root, dhampir, font):
    """造片段目录 + `asr/timeline.json`（放在**规范位置**）。"""
    clip = os.path.join(root, "output", "clips", "karaoke-clip")
    os.makedirs(os.path.join(clip, "data"), exist_ok=True)
    # ASR 的规范位置：<output_dir>/asr/ —— 这里 output_dir = root/output
    os.makedirs(os.path.join(root, "output", "asr"), exist_ok=True)

    mp4 = os.path.join(clip, "clip.mp4")
    r = run(["ffmpeg", "-v", "error", "-f", "lavfi", "-i",
             f"color=c=black:s=640x360:d={DUR},format=yuv420p",
             "-f", "lavfi", "-i", "anullsrc=r=48000:cl=stereo", "-shortest",
             "-r", str(FPS), "-c:v", "libx264", "-pix_fmt", "yuv420p", "-c:a", "aac",
             "-y", mp4])
    if r.returncode != 0:
        raise SystemExit(f"ffmpeg 造测试视频失败：{r.stderr[:400]}")

    with open(os.path.join(clip, "clip.srt"), "w", encoding="utf-8", newline="\n") as f:
        f.write(f"1\n00:00:00,000 --> 00:00:03,000\n{TEXT}\n\n")
    # `process` 的输入（`data/subtitles.json`）
    subs = [{"index": 1, "start": 0.0, "end": DUR, "text": TEXT, "html": TEXT}]
    with open(os.path.join(clip, "data", "subtitles.json"), "w", encoding="utf-8",
              newline="\n") as f:
        json.dump(subs, f, ensure_ascii=False)
    # ASR 字级时间轴（**规范位置**：片段目录上两级 + `asr/`）
    log = {"text": TEXT, "chars": len(TEXT), "times": TIMES,
           "breath_gap": 0.4, "breaths": []}
    with open(os.path.join(root, "output", "asr", "timeline.json"), "w",
              encoding="utf-8", newline="\n") as f:
        json.dump(log, f, ensure_ascii=False)

    with open(os.path.join(clip, "polish.toml"), "w", encoding="utf-8", newline="\n") as f:
        f.write(f"""config_version = 1
duration = {DUR}
processed = false

[source]
video = "clip.mp4"
subtitles = "clip.srt"
danmaku = ""
metadata = ""

[canvas]
orientation = "landscape"

[elements]
karaoke = true

[[segments]]
start = 0.0
end = {DUR}
fx = "low"
""")
    return clip


def bright_count(path, thr=40, step=3):
    from PIL import Image
    im = Image.open(path).convert("RGB")
    w, h = im.size
    px = im.load()
    c = 0
    for y in range(0, h, step):
        for x in range(0, w, step):
            r, g, b = px[x, y][:3]
            if r > thr and g > thr and b > thr:
                c += 1
    return c


def main():
    ap = argparse.ArgumentParser()
    ap.add_argument("--dhampir", default=os.path.join(
        REPO, "target", "debug", "dhampir", "bin", "dhampir.exe"))
    ap.add_argument("--font", default=None, help="一份含 CJK 的 .ttf/.otf")
    ap.add_argument("--keep", action="store_true")
    args = ap.parse_args()

    if sys.platform == "win32":
        sys.stdout.reconfigure(encoding="utf-8", errors="replace")

    if not os.path.exists(args.dhampir):
        raise SystemExit(f"找不到 Dhampir 二进制：{args.dhampir}（用 --dhampir 指定）")
    if not args.font or not os.path.exists(args.font):
        raise SystemExit("必须给 --font（底座不给字体就不出字幕，每帧都会是 0 像素）")
    try:
        import PIL  # noqa: F401
    except ImportError:
        raise SystemExit("需要 Pillow（python3 -m pip install pillow）")

    root = tempfile.mkdtemp(prefix="karaoke-pixels-")
    try:
        clip = build_fixture(root, args.dhampir, args.font)
        # 1) process：落 data/karaoke-cues.json（对齐结论）
        exe = os.path.join(REPO, "target", "debug", "yekitrim.exe")
        if not os.path.exists(exe):
            raise SystemExit(f"找不到 yekitrim.exe：{exe}")
        env = dict(os.environ, TMP=os.path.join(root, "tmp"))
        os.makedirs(env["TMP"], exist_ok=True)
        r = run([exe, "polish", "process", clip], env=env)
        cues = os.path.join(clip, "data", "karaoke-cues.json")
        if not os.path.exists(cues):
            raise SystemExit("process 没落 data/karaoke-cues.json（卡拉OK没生效）：\n"
                             + r.stdout[-1500:] + r.stderr[-800:])
        with open(cues, encoding="utf-8") as f:
            doc = json.load(f)
        print(f"✓ process：{len(doc['cues'])} 条字幕有结论，skipped={doc['skipped']}")

        # 2) 转译器：切 SRT 轨
        translator = os.path.join(REPO, "tools", "polish-to-dhampir.mjs")
        out_doc = os.path.join(root, "doc.json")
        r = run(["node", translator, clip, "--out", out_doc])
        if not os.path.exists(out_doc):
            raise SystemExit("转译器没出工程：\n" + r.stdout[-1500:])
        with open(out_doc, encoding="utf-8") as f:
            d = json.load(f)
        srt = [a["uri"] for a in d["assets"] if a.get("kind") == "subtitle"][0]
        with open(srt, encoding="utf-8") as f:
            srt_text = f.read()
        blocks = [b for b in re.split(r"\n\s*\n", srt_text) if b.strip()]
        print(f"✓ 转译器：字幕轨切成 {len(blocks)} 条 cue（期望 {len(TEXT)}）")
        if len(blocks) != len(TEXT):
            raise SystemExit(f"逐字切分条数不对：{len(blocks)} != {len(TEXT)}")

        # 字幕轨的淡入淡出必须是 0（否则逐字 cue 亮不起来）
        sub_track = next((t for t in d["timeline"]["tracks"] if t.get("id") == "sub"), None)
        st = (sub_track or {}).get("subtitle") or {}
        for k in ("fade_in_ms", "fade_out_ms", "rise_in_px", "rise_out_px"):
            if st.get(k) != 0:
                raise SystemExit(f"字幕轨 `{k}` 必须是 0（实测非 0 时有的字根本画不出来），"
                                 f"实得 {st.get(k)}")

        # 3) 真出帧
        frames = os.path.join(root, "frames")
        os.makedirs(frames, exist_ok=True)
        r = run([args.dhampir, "frame", "--project", out_doc,
                 "--from", "0", "--to", str(TOTAL_FRAMES - 1),
                 "--out", frames, "--font-file", args.font])
        got = len([f for f in os.listdir(frames) if f.endswith(".png")])
        if got != TOTAL_FRAMES:
            raise SystemExit(f"出帧数不对：{got} != {TOTAL_FRAMES}\n" + r.stdout[-800:])

        # 4) 判据：逐帧亮像素 → 每一步跃升都要落在某个字的字级时间戳上
        prof = {f: bright_count(os.path.join(frames, f"frame-{f:04d}.png"))
                for f in range(TOTAL_FRAMES)}
        steps = [f for f in range(1, TOTAL_FRAMES) if prof[f] != prof[f - 1]]
        print(f"✓ 出帧 {TOTAL_FRAMES} 张；亮像素跃升 {len(steps)} 次："
              f"{[round(s / FPS, 4) for s in steps]}")

        want = round(TIMES[-1] * FPS)  # 150
        if want not in steps:
            raise SystemExit(
                f"⚠️ **第 {len(TEXT)} 个字（{TARGET}）没有在它的字级时间戳 "
                f"({TIMES[-1]}s = 第 {want} 帧) 出现** —— "
                f"实际跃升在 {[round(s / FPS, 4) for s in steps]}")
        # 前 15 个字的跃升都在 2.5s 之前，且**最后一次**就是第 15 个字（它们的字级时间戳最晚 0.5s）。
        earlier = [s for s in steps if s < want]
        if not earlier:
            raise SystemExit(f"⚠️ {want} 帧之前一次跃升都没有 —— 前 15 个字没出现？")
        last15 = max(earlier)
        if last15 > round(TIMES[14] * FPS) + 2:
            raise SystemExit(
                f"⚠️ 前 15 个字的最后一次跃升在第 {last15} 帧，而它们的字级时间戳最晚是 "
                f"{TIMES[14]}s（第 {round(TIMES[14]*FPS)} 帧）—— 时间轴没跟着字级时间戳走")
        # ★★ **核心判据**：第 15 个字出齐之后到 2.5s 之间必须**恒定不变**
        #（= 第 16 个字**确实没有提前出现**，而前 15 个字也没闪）。
        plateau = {prof[f] for f in range(last15, want)}
        if len(plateau) != 1:
            raise SystemExit(
                f"⚠️ 第 {last15} 帧（前 15 个字出齐）到第 {want} 帧（{TARGET} 的字级时间戳）"
                f"之间画面**应当完全不变**，实得 {sorted(plateau)}")
        if prof[want] == prof[want - 1]:
            raise SystemExit(f"⚠️ 第 {want} 帧没有变化 ⇒ {TARGET} 没出现")
        print(f"✓ **{TARGET}** 在第 {want} 帧（{TIMES[-1]}s，它的字级时间戳）出现："
              f"{prof[want - 1]} -> {prof[want]} 亮像素")
        print(f"✓ 前 15 个字在第 {last15} 帧出齐（它们的字级时间戳最晚 {TIMES[14]}s），"
              f"之后到 {TIMES[-1]}s **恒定 {plateau.pop()} 像素** "
              f"⇒ {TARGET} 没有提前出现、也没有字闪掉")
        print("✓ 逐字切分的时间**跟着字级时间戳走**，不是均分"
              f"（均分会把第 {len(TEXT)} 个字放在 {DUR:.1f}s 附近而不是 {TIMES[-1]}s）")
        print("\nT5 真卡拉OK 端到端判据：全绿")
    finally:
        if args.keep:
            print(f"（产物保留在 {root}）")
        else:
            shutil.rmtree(root, ignore_errors=True)


if __name__ == "__main__":
    main()
