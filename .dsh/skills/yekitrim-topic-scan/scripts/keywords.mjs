/**
 * 关键词词表与匹配规则。
 *
 * 与 `scan.mjs` 解耦，方便加词：**加词只改这个文件**。
 *
 * 两类输入：
 *   ① 用户给的词（`--kw "恋爱,男友粉"`）—— 原样用，一个不加
 *   ② 没给词（`--discover`）—— 用下面的 `TOPIC_SUITE` 起手，再靠数据侧热度榜补
 *
 * 为什么不能只做"朴素子串"匹配（都是实测踩出来的）：
 *   - ASR 会把词听错（「男友粉」在原始稿里是残的，只有 LLM 订正稿才完整），
 *     所以词表要带**谐音/变体族**，报告里还必须贴原话让人自己判断；
 *   - `VR` 这种 2 字母 ASCII 词做朴素子串会命中一大片（`VRM`/`VRoid`/变量名），
 *     所以短 ASCII 词一律走**词边界**匹配；
 *   - 中文没有词边界概念，直接子串。
 */

/** 全 ASCII 的短词（≤3 字符）走词边界匹配，避免 `VR` 命中 `VRM` 这类。 */
const SHORT_ASCII = /^[a-z0-9]{1,3}$/

/**
 * 把一个关键词编译成匹配器。
 *
 * @param {string} kw 原始关键词
 * @returns {{kw: string, test: (text: string) => boolean, broad: boolean}}
 *   `broad` = 过宽的词（值得在报告里警告"这个词几乎什么都匹配"）
 */
export function compileKeyword(kw) {
  const raw = String(kw).trim()
  const lower = raw.toLowerCase()
  const isAscii = /^[\x20-\x7e]+$/.test(raw)

  if (isAscii && SHORT_ASCII.test(lower)) {
    // 词边界：两侧不能是字母/数字，否则 `vr` 会命中 `vrm`、`vrchat`
    const re = new RegExp(`(^|[^a-z0-9])${lower.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')}([^a-z0-9]|$)`, 'i')
    return { kw: raw, test: (t) => re.test(t), broad: lower.length <= 2 }
  }
  // 中文 / 长 ASCII：不区分大小写子串
  return {
    kw: raw,
    test: (t) => t.toLowerCase().includes(lower),
    broad: raw.length === 1,
  }
}

/** 一批关键词 → 匹配器数组（去重、去空）。 */
export function compileKeywords(list) {
  const seen = new Set()
  const out = []
  for (const k of list || []) {
    const raw = String(k).trim()
    if (!raw || seen.has(raw)) continue
    seen.add(raw)
    out.push(compileKeyword(raw))
  }
  return out
}

/**
 * 没给关键词时的起手词表（"题材套件"）。
 *
 * 分组只影响报告里的归类标题，不影响匹配。
 *
 * ⚠️ 两条纪律（都是实跑调出来的）：
 *   1. **别放过于通用的词**（`父母`/`害怕`/`一个人`/`弟弟` 这种）—— 12 个项目能命中 600+ 处，
 *      榜单直接被淹没，等于没有信息。词要"说了这句就知道在聊这个话题"。
 *   2. 这里只是**起手**：`--discover` 还会附一份**数据侧热度榜**，
 *      真正该蹭什么由热度榜 + 人工判断决定，不靠这张表穷举。
 */
export const TOPIC_SUITE = {
  '情感 / 婚恋': ['恋爱', '谈恋爱', '网恋', '相亲', '催婚', '彩礼', '分手', '前任', '暗恋', '表白', '告白', '暧昧', '早恋', '闪婚', '结婚'],
  '粉圈 / 偶像': ['男友粉', '女友粉', '妈妈粉', '老婆粉', '唯粉', '毒唯', '梦女', '私生', '站姐', '饭圈', '爱豆', '应援', '打榜', '追星', '塌房'],
  'VTuber / 圈子': ['维阿', 'virtuareal', 'vup', '虚拟主播', '中之人', '皮套', '开播', '切片'],
  '游戏 / 抽象梗': ['原神', '鸣潮', '明日方舟', '绝区零', 'steam', '抽卡', '保底', '上分', '坑队友', '抽象', '整活', '名场面', '逆天', '绷不住', '破防', '社死'],
}


/** 摊平成带分组的词项列表。 */
export function topicSuiteEntries() {
  const out = []
  for (const [group, kws] of Object.entries(TOPIC_SUITE)) {
    for (const kw of kws) out.push({ group, kw })
  }
  return out
}
