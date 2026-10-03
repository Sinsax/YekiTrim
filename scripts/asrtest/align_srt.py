"""v4: qwen3 文本 + sensevoice 时间轴 + LCS 锚点 + 锚点间局部 DP -> SRT

用法: uv run --with sherpa-onnx==1.13.3 --with numpy python align_srt.py <wav> <out.srt>
"""
import pathlib
import sys
import wave as w

import numpy as np
from sherpa_onnx import OfflineRecognizer

HOME = pathlib.Path.home() / ".yekitrim" / "models"
PUNCT = "\u3002\uFF01\uFF1F\uFF1B\uFF0C\u3001,.!?;:"


def fmt(t: float) -> str:
    ms = int(round(t * 1000))
    return f"{ms // 3600000:02d}:{ms % 3600000 // 60000:02d}:{ms % 60000 // 1000:02d},{ms % 1000:03d}"


def load_qwen3(samples, sr) -> str:
    m = HOME / "qwen3-asr"
    rec = OfflineRecognizer.from_qwen3_asr(
        conv_frontend=str(m / "conv_frontend.onnx"),
        encoder=str(m / "encoder.int8.onnx"),
        decoder=str(m / "decoder.int8.onnx"),
        tokenizer=str(m / "tokenizer"),
        num_threads=4, max_total_len=2048, max_new_tokens=512,
        temperature=1e-6, top_p=0.8, seed=42,
    )
    st = rec.create_stream()
    st.accept_waveform(sr, samples)
    rec.decode_stream(st)
    return st.result.text.strip()


def load_sensevoice(samples, sr) -> tuple[list[str], list[float]]:
    m = HOME / "sensevoice"
    rec = OfflineRecognizer.from_sense_voice(
        model=str(m / "model.int8.onnx"), tokens=str(m / "tokens.txt"),
        num_threads=4, language="zh", use_itn=True,
    )
    st = rec.create_stream()
    st.accept_waveform(sr, samples)
    rec.decode_stream(st)
    r = st.result
    chars, times = [], []
    for tok, ts in zip(r.tokens, [float(x) for x in r.timestamps]):
        t = tok.strip()
        if not t or t == "\uFFFD":
            continue
        for ch in t:
            chars.append(ch)
            times.append(ts)
    return chars, times


def lcs_anchors(q: list[str], c: list[str], min_run: int = 3) -> list[tuple[int, int, int]]:
    anchors = []
    i = 0
    while i < len(q):
        best_len, best_j = 0, -1
        for j in range(len(c)):
            k = 0
            while i + k < len(q) and j + k < len(c) and q[i + k] == c[j + k]:
                k += 1
            if k > best_len:
                best_len, best_j = k, j
        if best_len >= min_run:
            anchors.append((i, best_j, best_len))
            i += best_len
        else:
            i += 1
    return anchors


def align(qchars: list[str], schars: list[str], stimes: list[float]) -> list[float]:
    qtime: list[float | None] = [None] * len(qchars)
    anchors = lcs_anchors(qchars, schars)
    for qi, ci, ln in anchors:
        for k in range(ln):
            qtime[qi + k] = stimes[ci + k]

    # 锚点间局部 DP (时间窗 ±2s, 局部比例先验)
    seg_pts = [(0, 0)] + [(qi, ci) for qi, ci, ln in anchors] + [(len(qchars), len(schars))]
    INF = 10**9
    for k in range(len(seg_pts) - 1):
        a0, c0 = seg_pts[k]
        a1, c1 = seg_pts[k + 1]
        n, m = a1 - a0, c1 - c0
        if n <= 0 or m <= 0:
            continue
        lo, hi = stimes[c0], stimes[c1 - 1]
        dp = [[INF] * (m + 1) for _ in range(n + 1)]
        dp[0][0] = 0
        for i in range(n + 1):
            for j in range(m + 1):
                if i == 0 and j == 0:
                    continue
                best = INF
                if i > 0:
                    best = min(best, dp[i - 1][j] + 1)
                exp = lo + (i / n) * (hi - lo) if n > 0 else lo
                if j > 0 and abs(stimes[c0 + j - 1] - exp) <= 2.0:
                    best = min(best, dp[i][j - 1] + 1)
                if i > 0 and j > 0 and abs(stimes[c0 + j - 1] - exp) <= 2.0:
                    best = min(best, dp[i - 1][j - 1] + (0 if qchars[a0 + i - 1] == schars[c0 + j - 1] else 1))
                dp[i][j] = best
        i, j = n, m
        while i > 0 and j > 0:
            exp = lo + (i / n) * (hi - lo) if n > 0 else lo
            inwin = abs(stimes[c0 + j - 1] - exp) <= 2.0
            cand = [INF, INF, INF]
            cand[0] = dp[i - 1][j] + 1
            if inwin:
                cand[1] = dp[i][j - 1] + 1
                cand[2] = dp[i - 1][j - 1] + (0 if qchars[a0 + i - 1] == schars[c0 + j - 1] else 1)
            best = min(cand)
            if best == cand[2]:
                qtime[a0 + i - 1] = stimes[c0 + j - 1]
                i -= 1
                j -= 1
            elif best == cand[0]:
                i -= 1
            else:
                j -= 1

    # 剩余未锚定: 插值
    for i in range(len(qtime)):
        if qtime[i] is None:
            prev = [t for t in qtime[:i] if t is not None]
            nxt = [t for t in qtime[i + 1:] if t is not None]
            if prev and nxt:
                qtime[i] = (prev[-1] + nxt[0]) / 2
            elif prev:
                qtime[i] = prev[-1]
            else:
                qtime[i] = 0.0
    return [float(x) for x in qtime]


def build_srt(qtext: str, qtime: list[float], qnorm_idx: list[int], pos_map: dict[int, int]) -> str:
    MAX_DUR, MIN_DUR, MAX_GAP = 10.0, 2.0, 1.2
    units = []
    buf, oi = "", 0
    for i, ch in enumerate(qtext):
        buf += ch
        if ch in PUNCT and buf.strip():
            units.append((buf, i - len(buf) + 1))
            buf = ""
    if buf.strip():
        units.append((buf, len(qtext) - len(buf)))

    def unit_range(u):
        text, oi = u
        qis = [k for k in qnorm_idx if oi <= k < oi + len(text)]
        if not qis:
            return None, None
        return qtime[pos_map[qis[0]]], qtime[pos_map[qis[-1]]] + 0.3

    segs = []
    cur = []
    seg_start = 0.0

    def close(end):
        if cur:
            text = "".join(u[0] for u in cur)
            if text.strip():
                segs.append((seg_start, end, text))

    for u in units:
        t0, t1 = unit_range(u)
        if t0 is None:
            continue
        if not cur:
            seg_start = t0
            cur.append((u[0], u[1], t1))
            continue
        prev_end = cur[-1][2]
        if (t1 - seg_start > MAX_DUR) or (t0 - prev_end > MAX_GAP):
            close(prev_end)
            seg_start = t0
            cur = [(u[0], u[1], t1)]
        else:
            cur.append((u[0], u[1], t1))
    if cur:
        close(cur[-1][2])

    merged = []
    for s, e, t in segs:
        if merged and e - s < MIN_DUR:
            ps, pe, pt = merged[-1]
            merged[-1] = (ps, e, pt + t)
        else:
            merged.append((s, e, t))

    lines = []
    prev = 0.0
    for i, (s, e, t) in enumerate(merged, 1):
        s = max(s, prev)
        e = max(e, s + 0.3)
        prev = e
        lines.append(f"{i}\n{fmt(s)} --> {fmt(e)}\n{t}\n")
    return "\n".join(lines)


def main() -> None:
    wav_path = pathlib.Path(sys.argv[1])
    out_path = pathlib.Path(sys.argv[2])
    with w.open(str(wav_path), "rb") as fh:
        sr = fh.getframerate()
        raw = fh.readframes(fh.getnframes())
    samples = np.frombuffer(raw, dtype=np.int16).astype(np.float32) / 32768.0

    qtext = load_qwen3(samples, sr)
    schars, stimes = load_sensevoice(samples, sr)
    qnorm_idx = [i for i, ch in enumerate(qtext) if ch not in PUNCT and not ch.isspace()]
    qchars = [qtext[i] for i in qnorm_idx]
    pos_map = {idx: k for k, idx in enumerate(qnorm_idx)}

    qtime = align(qchars, schars, stimes)
    srt = build_srt(qtext, qtime, qnorm_idx, pos_map)
    out_path.write_bytes(b"\xEF\xBB\xBF" + srt.encode("utf-8"))
    n_segs = len([l for l in srt.splitlines() if " --> " in l])
    print(f"qwen3 chars={len(qchars)} sensevoice chars={len(schars)} segs={n_segs}")
    print(f"saved: {out_path}")


if __name__ == "__main__":
    main()
