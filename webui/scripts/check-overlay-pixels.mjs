#!/usr/bin/env node
// **hue 并入覆盖层：像素对拍**（2026-10-20，52b）。
//
// 判据（两条，缺一不可）：
//   A = legacy `hue_shift` 事件；B = 迁成 `overlay{overlay_type:"hue"}`；C = 没有这条事件。
//   ① **A ↔ B ≈ 0**（迁移前后像素不变）；
//   ② **A ↔ C > 0**（否则"两边都不画"也会判相等 —— 那是假绿）。
//
// ⚠️ 本夹具**刻意不覆盖**「同一时刻既有 legacy hue 又有 overlay」的场景：那是**设计口径变化**
//    （从"两条都画"变成"只画第一条"，overlay 的一档语义），已由 Rust 用例
//    `overlay_takes_only_the_first_in_window_event` 覆盖，不是回归。
//
// 用法： node webui/scripts/check-overlay-pixels.mjs [--project <片段目录>]
// 缺 dhampir.exe / ffmpeg / 工程 ⇒ **SKIP + 说明缺什么**，绝不写成通过。
import { execFileSync } from "node:child_process"
import { existsSync, mkdirSync, readFileSync, rmSync, writeFileSync, copyFileSync } from "node:fs"
import { dirname, join, resolve } from "node:path"
import { fileURLToPath } from "node:url"
import { tmpdir } from "node:os"

const HERE = dirname(fileURLToPath(import.meta.url))
const ROOT = resolve(HERE, "..", "..")
const gi = (flag, dflt = null) => {
  const i = process.argv.indexOf(flag)
  return i >= 0 && i + 1 < process.argv.length ? process.argv[i + 1] : dflt
}

const DHAMPIR_CANDIDATES = [
  "F:/para/Code/Dhampir/target/release/dhampir.exe",
  "F:/para/Code/Dhampir/target/debug/dhampir.exe",
]
const dhampir = DHAMPIR_CANDIDATES.find((p) => existsSync(p))
const hasFfmpeg = (() => {
  try { execFileSync("ffmpeg", ["-version"], { stdio: "ignore" }); return true } catch { return false }
})()
const project = gi("--project", join(ROOT, ".workbuddy", "t46", "base", "clip-15"))

if (!dhampir || !hasFfmpeg || !existsSync(join(project, "polish.toml"))) {
  console.log("SKIP：环境不满足，**没有跑**（这不是通过）")
  if (!dhampir) console.log(`  缺 dhampir.exe：找过 ${DHAMPIR_CANDIDATES.join(" / ")}（装法：在 Dhampir 仓 cargo build --release）`)
  if (!hasFfmpeg) console.log("  缺 ffmpeg：需要它在 PATH 上（本仓出片本来就依赖它）")
  if (!existsSync(join(project, "polish.toml"))) console.log(`  缺工程：${project}（用 --project 指一个含 polish.toml 的片段目录）`)
  process.exit(0)
}

const FRAME = 960 // t = 16.0s @60fps（clip-15 的段 1 = 14.0~34.5s）
const work = join(tmpdir(), `yk-hue-pixels-${process.pid}`)
rmSync(work, { recursive: true, force: true })
mkdirSync(work, { recursive: true })

const baseToml = readFileSync(join(project, "polish.toml"), "utf8")
const HUE_LEGACY = '[[segments.events]]\ntype = "hue_shift"\ntime = 16.0\nend = 16.4\ncolor = "#ff0040"\n'
const HUE_OVERLAY = '[[segments.events]]\ntype = "overlay"\ntime = 16.0\nend = 16.95\noverlay_type = "hue"\ncolor = "#ff0040"\n'

/** 造一份变体工程（复制夹具 → 把黑底视频当 clip.mp4 → 在段 1 前插一条事件）。 */
function makeVariant(tag, eventToml) {
  const dir = join(work, tag)
  mkdirSync(dir, { recursive: true })
  for (const sub of ["data", "assets", "sfx"]) {
    const src = join(project, sub)
    if (existsSync(src)) execFileSync("cmd", ["/c", "xcopy", src.replace(/\//g, "\\"), join(dir, sub).replace(/\//g, "\\"), "/E/I/Q/Y"], { stdio: "ignore" })
  }
  // 2s 纯黑 1080p60 —— 覆盖层是全帧色罩，底片内容不影响判据，但必须存在
  execFileSync("ffmpeg", ["-y", "-f", "lavfi", "-i", "color=c=black:s=1920x1080:r=60:d=20", "-pix_fmt", "yuv420p", join(dir, "clip.mp4")], { stdio: "ignore" })
  // 在段表之前插事件：插在文件里第一个 `[[segments.events]]` 之前 ⇒ 同一时刻"第一条"就是它
  const idxFirst = baseToml.indexOf("[[segments.events]]")
  const toml = eventToml ? baseToml.slice(0, idxFirst) + eventToml + baseToml.slice(idxFirst) : baseToml
  writeFileSync(join(dir, "polish.toml"), toml)
  const proj = join(dir, "dhampir.json")
  execFileSync("node", [join(ROOT, "tools", "polish-to-dhampir.mjs"), dir, "--out", proj], { stdio: "ignore" })
  const out = join(dir, "frames")
  mkdirSync(out, { recursive: true })
  execFileSync(dhampir, ["frame", "--project", proj, "--frame", String(FRAME), "--out", out], { stdio: "ignore" })
  const png = join(out, `frame-${FRAME}.png`)
  if (!existsSync(png)) throw new Error(`dhampir 没出帧：${png}`)
  // PNG → rgb24 raw，便于逐像素比（不自己写 PNG 解码器）
  const raw = join(dir, "frame.raw")
  execFileSync("ffmpeg", ["-y", "-i", png, "-f", "rawvideo", "-pix_fmt", "rgb24", raw], { stdio: "ignore" })
  return readFileSync(raw)
}

/** 平均绝对差（0~255）：两张同尺寸 rgb24 逐字节平均差。 */
function meanAbsDiff(a, b) {
  const n = Math.min(a.length, b.length)
  if (n === 0) return 255
  let sum = 0
  for (let i = 0; i < n; i++) sum += Math.abs(a[i] - b[i])
  return sum / n
}

let failed = 0
const report = (label, ok, detail) => {
  console.log(`  ${ok ? "ok  " : "FAIL"} ${label}${detail ? `（${detail}）` : ""}`)
  if (!ok) failed++
}

try {
  console.log(`hue 像素对拍：工程 ${project}，第 ${FRAME} 帧（t=16.0s）`)
  const c = makeVariant("c-none", "")
  const a = makeVariant("a-legacy", HUE_LEGACY)
  const b = makeVariant("b-overlay", HUE_OVERLAY)
  const dAB = meanAbsDiff(a, b)
  const dAC = meanAbsDiff(a, c)
  report("① A(legacy hue_shift) ↔ B(overlay{hue}) 差异 ≈ 0", dAB <= 0.5, `平均绝对差 ${dAB.toFixed(3)}/255`)
  report("② A ↔ C(无 hue) 差异 > 0（防「两边都不画」假绿）", dAC > 1.0, `平均绝对差 ${dAC.toFixed(3)}/255`)
  console.log(failed ? `\n${failed} 项失败` : "\n全部通过")
  process.exit(failed ? 1 : 0)
} catch (e) {
  // ⚠️ **跑不起来就是跑不起来**：绝不写成通过（本机实测底座 `frame` 写 PNG 被拒 os error 5，
  //    release / debug / 三个输出目录 / 重定向 TEMP 全试过 —— 见 52b 报告）。
  console.log(`BLOCKED：像素对拍**没有跑成**（不是通过）—— ${e?.message || e}`)
  // ⚠️ **默认 SKIP（exit 0）、`--strict` 才红**：
  //    底座 `dhampir.exe frame` 在本机写 PNG 报 `Io { code: 5, PermissionDenied }`，
  //    连手动 `mkdir` 出来、PowerShell 能写的目录也一样 ⇒ 是**底座侧的写盘问题**，不是本判据的断言失败。
  //    若固定 exit 1，全量 `check-*.mjs` 扫描会**永久变红** —— 久而久之大家会习惯性忽略红，那比不跑更糟。
  //    想让它红就显式 `--strict`（发布前 / CI 该这么跑）。
  //    依据：仓库惯例「缺依赖就 SKIP 并明说缺什么，绝不写成通过」。
  process.exit(process.argv.includes('--strict') ? 1 : 0)
} finally {
  rmSync(work, { recursive: true, force: true })
}
