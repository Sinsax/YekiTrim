// audio.js — SFX 播放器，从 templates/index.html 的 sfxPlay/curveAdjust/scheduleSound 移植。
// 音量跟随视频响度曲线（与渲染混音同思路）：视频吵 → 压低音效，安静 → 正常。
// 主音量 localStorage 记忆（渲染不受影响，仅预览）。

const MASTER_VOL_KEY = 'yekitrim_master_vol'
const MUTED_KEY = 'yekitrim_muted'

/**
 * 全应用共用的主音量读写（精修预览与切片页的原片预览都用这一份，别各写一套）。
 *
 * ⚠️ 读音量 **不能写 `parseFloat(...) || 1`**：音量 0 是合法值，falsy 判定会把它读回 1
 * （用户实测：滑到 0 刷新后又响了）。统一用 isFinite + clamp。
 */
export function readMasterVolume() {
  try {
    const saved = parseFloat(localStorage.getItem(MASTER_VOL_KEY))
    if (Number.isFinite(saved)) return Math.min(1, Math.max(0, saved))
  } catch {}
  return 1
}

export function readMuted() {
  try {
    return localStorage.getItem(MUTED_KEY) === '1'
  } catch {
    return false
  }
}

export function saveMasterVolume(v) {
  const n = Number(v)
  try {
    localStorage.setItem(MASTER_VOL_KEY, String(Number.isFinite(n) ? Math.min(1, Math.max(0, n)) : 1))
  } catch {}
}

export function saveMuted(m) {
  try {
    localStorage.setItem(MUTED_KEY, m ? '1' : '0')
  } catch {}
}

export function createSfxPlayer({ resolveUrl, onError } = {}) {
  let masterVol = readMasterVolume()
  let muted = readMuted()

  const sfxCache = new Map() // name → Audio
  const loadFailed = new Set() // 加载失败已上报过的 name（每个只报一次，别刷屏）
  let currentT = 0 // 当前播放时间（音量跟随曲线用）
  // ⚠️ **基线另设哨兵**，不能用 `currentT` 的初值 0 当"第一次报时刻"的判据：
  // 0 是个合法时刻，而预览一进来就可能报一个较大的 t —— 那会把开头之前的音效全部倒放出来。
  let syncBaseline = null // null = 还没建立基线（第一次 sync / 刚 setEvents）
  let curve = []

  function setLoudnessCurve(points) {
    curve = Array.isArray(points) ? points : []
  }

  // 查响度曲线 → 目标: 音效比视频低 8dB（文件响度按默认 -14）
  function curveAdjust(t) {
    if (!curve.length) return 1.0
    let db = -26.0
    for (let i = curve.length - 1; i >= 0; i--) {
      if (curve[i].t <= t) { db = curve[i].db; break }
    }
    if (db < -100) return 1.0
    const f = Math.pow(10, (db - 8.0 + 14.0) / 20)
    return Math.max(0.1, Math.min(f, 4.0))
  }

  function obtain(name) {
    let a = sfxCache.get(name)
    if (a && !a.ended && a.currentTime > 0) {
      // 已在播放：clone 新实例（复制 src，浏览器复用缓存），不打断前一次
      a = a.cloneNode()
    } else if (a) {
      a.currentTime = 0
    } else {
      a = new Audio(resolveUrl ? resolveUrl(name) : 'sfx/' + name + '.mp3')
      sfxCache.set(name, a)
    }
    return a
  }

  function play(name, vol, dur) {
    if (!name) return
    vol = vol || 0.04
    const target = vol * curveAdjust(currentT) * (muted ? 0 : masterVol)
    const a = obtain(name)
    // 加载失败（404 / 解码不了）以前是**纯静默** —— "预览没有音效"连个可查的痕迹都没有。
    // cloneNode 出来的实例不带原监听，所以在每次拿到实例时都补挂（failed 去重）。
    if (!loadFailed.has(name)) {
      a.addEventListener('error', () => {
        if (loadFailed.has(name)) return
        loadFailed.add(name)
        try { onError && onError(name) } catch {}
      }, { once: true })
    }
    a.volume = 0
    a.play().catch(() => {})
    // ~30ms 缓入，消除爆音
    let steps = 0
    const fade = setInterval(() => {
      steps++
      a.volume = Math.min(target, target * steps / 5)
      if (steps >= 5 || a.ended) clearInterval(fade)
    }, 6)
    // 限时停止：事件时长 + 尾音余量（渲染路径 atrim 等价行为）
    const stopAt = Math.min(dur || 1.5, 30)
    if (stopAt > 0) {
      setTimeout(() => {
        const out = setInterval(() => {
          a.volume = Math.max(0, a.volume - 0.03)
          if (a.volume <= 0.005 || a.ended) {
            clearInterval(out)
            try { a.pause(); a.currentTime = 0 } catch {}
          }
        }, 20)
      }, (stopAt + 0.2) * 1000)
    }
  }

  // 逐帧驱动：soundEvents 为 buildIndex 产出的音效事件表
  // 触发窗口 0.12s；seek 后对未到达事件重新武装，支持回退重触发
  // ⚠️ **窗口不能写死 120ms**（2026-10-02 实测："有些音效播了、有些没有"）。
  //
  // 旧写法是 `e.time <= t && e.time > t - 0.12`，即"事件必须正好落进这一帧往回 120ms 的窗口"。
  // 它有两个隐含前提，两个都不成立：
  //   ① 时钟**逐帧连续推进** —— 但预览有"跳过屏蔽区间"与 A/B 换台：命中时
  //      `v.currentTime` 会**一次跳过整段**（几百毫秒到几十秒），落在被跳过区间里的音效
  //      没有任何一帧的窗口能覆盖它 ⇒ **永久丢失**（`fired` 是粘性的，错过不再来）。
  //   ② **每一帧都会调到 sync** —— 但 Dhampir 分支有好几处提前 `return`（预滚命中、
  //      换台、定格遮罩），帧到了、sync 却没跑。
  // 改成"从上次报过的时刻到这次之间的**全部**事件都要响"（左开右闭），
  // 对丢帧、跳段、暂停恢复都成立；代价只是极短时间内的连击音效会挤在一帧里发出，
  // 而那本来就是同一瞬间该响的东西。
  //
  // 拖进度条/seek 不该"补响"中间的音效 —— 由 `seek()` 重置基线把它挡掉（见下）。
  function sync(soundEvents, t) {
    currentT = t
    if (!soundEvents || !soundEvents.length) return
    // 第一次报时刻（或换了一批事件）只建基线，不倒放历史音效。
    if (syncBaseline == null) { syncBaseline = t; return }
    // 往回跳（用户拖进度条但没走 seek() 那条路）：只重置，不补响。
    if (t < syncBaseline) {
      syncBaseline = t
      for (const e of soundEvents) { if (e.time >= t) e.fired = false }
      return
    }
    for (const e of soundEvents) {
      if (!e.fired && e.time > syncBaseline && e.time <= t) {
        e.fired = true
        play(e.name, e.volume, e.duration)
      }
    }
    syncBaseline = t
  }

  function seek(t) {
    currentT = t
    // seek = 用户主动跳转：**重新建立基线**，否则下一次 sync 会把这段的中间音效倒放出来
    // （"拖进度条之后炸出一串音效"就是这么来的）。
    syncBaseline = t
    if (soundEventsCache) for (const e of soundEventsCache) {
      if (e.time >= t) e.fired = false
    }
  }

  let soundEventsCache = null

  function setEvents(events) {
    if (events !== soundEventsCache) {
      soundEventsCache = events
      // 换了一批事件：基线作废（下一帧重新建立），否则新事件会被拿去和旧时刻相减。
      syncBaseline = null
      if (events) for (const e of events) e.fired = false
    }
  }

  function getMasterVolume() { return masterVol }
  function isMuted() { return muted }
  function setMasterVolume(v) {
    masterVol = v
    saveMasterVolume(v)
    saveMuted(muted)
  }
  function setMuted(m) {
    muted = m
    saveMasterVolume(masterVol)
    saveMuted(muted)
  }

  return { play, sync, seek, setEvents, setLoudnessCurve, getMasterVolume, isMuted, setMasterVolume, setMuted }
}
