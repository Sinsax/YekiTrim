/**
 * 热度曲线（时间轴底部「时间」标尺带）—— **唯一一份口径**。
 *
 * # 它是什么
 *
 * 把「这一刻有多少人在刷 / 说了多少字」画成一条曲线，叠在时间轴标尺上，
 * 让"哪一段值得剪"在编辑时一眼看得见（定稿于 2026-10：**弹幕按条、字幕按字量**）。
 *
 * # 两条输入、两种量纲
 *
 * | 输入 | 来源 | 怎么数 |
 * |---|---|---|
 * | 字幕 | `data/subtitles.json`（`polish init` / `parse` 解析出来的那份） | **字量**：码点数（去空白），按与桶的重叠秒数摊开 |
 * | 弹幕 | `data/danmaku.json`（同一趟解析出来的那份） | **条数**：1 条算 1，按 `start` 归桶 |
 *
 * # 与 `process/punch.rs` 的关系（**读之前先看这一段**）
 *
 * 母口径在 Rust 那边：`punch::subtitle_signal`（按字）、`punch::danmaku_signal`（按条）、
 * `normalize_points`（各自按**本片段最大值**归一）。本模块**沿用**这三条，但有两处**有意不同**，
 * 写在下面两段里 —— 别把"不一样"当成漂移去改齐：
 *
 * ① **窗口更细**（`HEAT_WINDOW_SECS` = 0.5s）：`PUNCH_WINDOW_SECS` 是 **5s**，
 *    那是**判据窗**（判"这里是不是爆点"，窗要够宽才不被一句话切散）。
 *    而画曲线要的是形状，5s 桶在 60px/s 下是 300px 一个台阶，根本不叫曲线。
 * ② **权重反过来**（弹幕 0.6 / 字幕 0.4）：爆点判据必须"**无弹幕也能工作**"，
 *    所以规格把字幕定成主判据（0.6）、弹幕是次（0.2）—— 那是**判据**的正确性要求。
 *    这里只服务观感：弹幕是"当场有多少人真的在反应"的直接证据，用户口径 2026-10 明确要**弹幕权重大一点**。
 *
 * ⚠️ **共同的那半截（按字 / 按条 / 自基准归一 / 缺席的那路不参与归一）由
 * `webui/scripts/check-heat-curve.mjs` 从 `punch.rs` 源码里逐值对拍** —— 那边改了而这边没跟，门禁会红。
 *
 * # 三条容易写错、都踩过的细节
 *
 * 1. **字数按码点算**：JS 的 `String.length` 是 UTF-16 码元数，emoji 会算 **2**，
 *    而全仓（Rust `chars().count()`）算 **1**。本仓 emoji 密度极高（目录 881 条），
 *    用 `.length` 就是"带 emoji 的字幕被凭空放大"。
 * 2. **字量按"与桶的重叠"摊开**，不是按起点归桶、更**不是**"字数 ÷ 字幕时长"：
 *    - 按起点归桶 ⇒ 一句 3 秒的话只在开头那一桶冒尖；
 *    - 按 `字数 ÷ 时长` ⇒ 被 ASR 碎片劫持 —— 实测 0.05s 一条的碎片能算出 **60 字/秒**
 *      （`punch.rs` 的 `subtitle_signal_weights_characters_not_fragments` 就是为这个坑写的）。
 *    摊开之后一条 0.05s / 3 字的碎片在 0.5s 桶里只贡献 **0.3**。
 * 3. **缺席的那一路不参与归一化**（不是"当 0 分"）：没有弹幕时若按 0 分算，
 *    字幕判出的高热度会被整体拉低 —— 那正是 punch 那边写明的"静默失效"。
 *    所以没有弹幕时权重整体转到字幕（字幕独占 1.0）。
 *
 * # 快照，不是实时
 *
 * 用户口径（2026-10）：**按 `polish init` / 装载时的数据算一次就行，不随编辑重算**。
 * 所以调用方只在 `loadData()` 里建一次这份序列并**冻住**；拖动字幕块不该让它变
 * （那也顺带省掉了每次拖拽重算全片桶的开销）。
 */

/** 桶宽（秒）。**比判据窗细**（`punch::PUNCH_WINDOW_SECS` = 5s），理由见模块头 ①。 */
export const HEAT_WINDOW_SECS = 0.5

/** 合成权重：弹幕为主（用户口径 2026-10「弹幕权重大一点」；与判据窗的 0.6/0.2 是两回事）。 */
export const HEAT_W_DANMAKU = 0.6
export const HEAT_W_SUBTITLE = 0.4

/**
 * **显示**对比拉伸的指数 —— 只影响「画多高 / 读多少」，**不改 `heat` 的数值**（对拍那套不受影响）。
 *
 * 为什么要它（2026-10 实测）：字幕那一路的桶值是"这条字幕说话时的**字/秒**"，而它在一条字幕内是
 * **常数** ⇒ 原始曲线天生是一串**台阶**，台阶之间又只有两三成落差，看着就像一条平线
 * （实测某片段：`1 1 1 1 1 1 0.48 0 0.32 0.33×5 0.21 0 …`）。平方把落差拉开
 * （0.33 → 0.11、0.6 → 0.36、1.0 → 1.0），**峰值照样顶格** ⇒ 起伏一眼看得见。
 * ⚠️ 读数与曲线**必须走同一个映射**（否则界面在说谎）：`heatDisplay` 是它俩唯一的入口。
 */
export const HEAT_DISPLAY_GAMMA = 2

/** 把 0~1 的热度映射成**画线/读数**用的 0~1（见 `HEAT_DISPLAY_GAMMA`）。钳到 [0,1]、非数字当 0。 */
export function heatDisplay(v, gamma = HEAT_DISPLAY_GAMMA) {
  const x = Math.max(0, Math.min(1, Number(v) || 0))
  const g = Number(gamma) > 0 ? Number(gamma) : 1
  return Math.pow(x, g)
}

/**
 * 码点数（**去空白**）—— 与 Rust `chars().filter(|c| !c.is_whitespace()).count()` 同口径。
 *
 * ⚠️ 必须走 `for…of`（按码点迭代）：`String.length` 数的是码元，`'😂'.length === 2`。
 */
export function heatCharCount(text) {
  let n = 0
  for (const ch of String(text == null ? '' : text)) {
    if (!/\s/.test(ch)) n++
  }
  return n
}

/**
 * 按桶算出热度序列。**纯函数**：不读文件、不碰 DOM，输入一样输出就一样（可单测、可对拍）。
 *
 * @param {object} input
 * @param {Array<{start:number,end?:number,text?:string}>} [input.subtitles] 生效字幕（`data/subtitles.json` 的形状）
 * @param {Array<number>} [input.danmakuStarts] 弹幕的 `start`（秒）—— **只留这个字段**，文本/颜色与本曲线无关
 * @param {number} [input.duration] 片段时长（秒）`polish.toml` 的 `duration`
 * @param {number} [input.windowSecs] 桶宽，缺省 `HEAT_WINDOW_SECS`
 * @returns {{windowSecs:number,count:number,sub:number[],dm:number[],heat:number[],
 *            peak:number,peakAt:number,hasDanmaku:boolean,hasSubtitle:boolean}}
 *   `sub` = 每桶字/秒，`dm` = 每桶条数，`heat` = 合成后的 0~1（**两路各自按本片段最大值归一**后再加权）。
 *   `peak`/`peakAt` 给调用方画"相对本片最热那一刻"的百分比与读数用。
 */
export function heatSeries(input) {
  const src = input || {}
  const subtitles = Array.isArray(src.subtitles) ? src.subtitles : []
  const danmakuStarts = Array.isArray(src.danmakuStarts) ? src.danmakuStarts : []
  const winRaw = Number(src.windowSecs)
  const win = Number.isFinite(winRaw) && winRaw > 0 ? winRaw : HEAT_WINDOW_SECS
  const durRaw = Number(src.duration)
  const dur = Number.isFinite(durRaw) && durRaw > 0 ? durRaw : 0
  // 至少一桶：duration 拿不到（老工程/读取失败）时也要给出一条"平"的序列，而不是空对象 ——
  // 调用方那边 `count === 0` 与"没有数据"长得一样，画线时又得再判一次。
  const n = Math.max(1, Math.ceil(dur / win))
  const sub = new Array(n).fill(0)
  const dm = new Array(n).fill(0)

  for (const s of subtitles) {
    const chars = heatCharCount(s && s.text)
    if (chars <= 0) continue
    const a = Number(s && s.start)
    if (!Number.isFinite(a)) continue
    const start = Math.max(0, a)
    let end = Number(s && s.end)
    if (!Number.isFinite(end) || end <= start) {
      // 点事件（缺 end / 零长）：整份落在起点那一桶 —— 与 punch 的"起点归窗"同口径。
      // ⚠️ 不能走下面的重叠分支：重叠秒数会是 0，这条字幕的字数就**静默消失**了。
      const i = Math.floor(start / win)
      if (i >= 0 && i < n) sub[i] += chars
      continue
    }
    const first = Math.floor(start / win)
    const last = Math.floor((end - 1e-9) / win)
    for (let i = Math.max(0, first); i <= Math.min(n - 1, last); i++) {
      const b0 = i * win
      const b1 = b0 + win
      const ov = Math.min(end, b1) - Math.max(start, b0)
      if (ov <= 0) continue
      sub[i] += (chars * ov) / win
    }
  }

  for (const t of danmakuStarts) {
    const v = Number(t)
    if (!Number.isFinite(v) || v < 0) continue
    const i = Math.floor(v / win)
    if (i >= 0 && i < n) dm[i] += 1
  }

  let maxSub = 0
  for (const v of sub) if (v > maxSub) maxSub = v
  let maxDm = 0
  for (const v of dm) if (v > maxDm) maxDm = v

  // 自基准归一 + **缺席的那路不参与归一化**（见模块头 ③）
  let wDm = HEAT_W_DANMAKU
  let wSub = HEAT_W_SUBTITLE
  if (maxDm <= 0) {
    wDm = 0
    wSub = 1
  } else if (maxSub <= 0) {
    wSub = 0
    wDm = 1
  }

  const heat = new Array(n)
  let peak = 0
  let peakAt = 0
  for (let i = 0; i < n; i++) {
    const v = wSub * (maxSub > 0 ? sub[i] / maxSub : 0) + wDm * (maxDm > 0 ? dm[i] / maxDm : 0)
    heat[i] = v
    if (v > peak) {
      peak = v
      peakAt = i * win
    }
  }

  return { windowSecs: win, count: n, sub, dm, heat, peak, peakAt, hasDanmaku: maxDm > 0, hasSubtitle: maxSub > 0 }
}

/**
 * 取 `t` 时刻所在的桶（读数用）。越界钳到首/末桶 —— 播放头可能落在 duration 之外
 * （后处理会改写段尾），返回 `null` 会让读数在片尾闪一下空白。
 */
export function heatBucketAt(series, t) {
  if (!series || !series.count) return null
  const win = series.windowSecs > 0 ? series.windowSecs : HEAT_WINDOW_SECS
  const v = Number(t)
  let i = Number.isFinite(v) && v > 0 ? Math.floor(v / win) : 0
  if (i < 0) i = 0
  if (i > series.count - 1) i = series.count - 1
  return {
    index: i,
    start: i * win,
    end: (i + 1) * win,
    heat: series.heat[i],
    sub: series.sub[i],
    dm: series.dm[i],
  }
}
