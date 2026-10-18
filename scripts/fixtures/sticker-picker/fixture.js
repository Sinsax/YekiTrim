// 贴纸选择器 headless 夹具数据（形状照真接口，别改成"更干净"的形状）：
//   GET /api/polish/sticker-library → { packs: [{name, built, description, tags:[{tag,files}], files}] }
//   GET /api/polish/emoji-catalog   → { groups: [{tag, codepoints:[hex]}] }
// 用途：scripts/check-sticker-picker-layout.mjs —— 真 Chrome 里挂真组件量几何。
function mkFiles(prefix, n) { const a = []; for (let i = 1; i <= n; i++) a.push(prefix + '_' + i + '.gif'); return a; }
function union(list) { const seen = new Set(); const out = []; for (const f of list) if (!seen.has(f)) { seen.add(f); out.push(f); } return out; }

export const LIB = [
  { name: 'ysyk', built: true, description: '有兽焉表情包（本地库，已构建）', tags: [
    { tag: '害怕', files: mkFiles('害怕', 12) }, { tag: '开心', files: mkFiles('开心', 9) },
    { tag: '无语', files: mkFiles('无语', 11) }, { tag: '生气', files: mkFiles('生气', 8) },
    { tag: '疑问', files: mkFiles('疑问', 7) }, { tag: '鼓掌', files: mkFiles('鼓掌', 6) },
    { tag: '比心', files: mkFiles('比心', 5) }, { tag: '哭泣', files: mkFiles('哭泣', 14) },
  ], extra: ['杂_1.gif', '杂_2.gif'] },
  { name: 'emotelab', built: true, description: 'EmoteLab 导出的通用表情', tags: [
    { tag: '日常', files: mkFiles('daily', 22) }, { tag: '情绪', files: mkFiles('mood', 18) },
    { tag: '道具', files: mkFiles('prop', 11) },
  ], extra: [] },
  { name: 'meme-26', built: false, description: '', tags: [], extra: mkFiles('meme', 24) },
];
for (const p of LIB) {
  let all = [];
  for (const t of p.tags) all = all.concat(t.files);
  p.files = union(all.concat(p.extra || []));
  delete p.extra;
}

export const EMOJI = [
  { tag: '笑脸', codepoints: ['1f600','1f602','1f605','1f606','1f609','1f60a','1f60d','1f618','1f61c','1f929','1f92a','1f973','1f970','1f60e','1f917','1f642'] },
  { tag: '手势', codepoints: ['1f44d','1f44f','1f64c','1f44c','270c','1f918','1f91d','1f44b','1f91e','1f440','1f4aa','1f64f'] },
  { tag: '心情', codepoints: ['1f622','1f62d','1f621','1f631','1f633','1f914','1f644','1f971','1f62e','1f634','1f92f','1f973'] },
];
