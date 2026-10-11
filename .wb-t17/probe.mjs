import { mkdtempSync, writeFileSync, readFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { execFileSync } from 'node:child_process';
import { pathToFileURL } from 'node:url';
import { resolve } from 'node:path';

const mod = await import(pathToFileURL(resolve('tools/polish-to-dhampir.mjs')).href);

const NEW = ['scale_in_from','scale_in_peak','scale_in_ms','shake_px','shake_hz','shake_phase','plate_color','plate_pad_px'];
const NL = String.fromCharCode(10);

function run(tag, elementsLines) {
  const dir = mkdtempSync(join(tmpdir(), 't17-'));
  execFileSync('ffmpeg', ['-v','error','-f','lavfi','-i','color=c=black:s=320x240:d=2,format=yuv420p','-r','30','-c:v','libx264','-pix_fmt','yuv420p','-y',join(dir,'clip.mp4')],{stdio:'ignore'});
  writeFileSync(join(dir,'clip.srt'), ['1','00:00:00,300 --> 00:00:01,200','有字幕','', ''].join(NL),'utf8');
  const L = [
    'config_version = 1','duration = 2.0','processed = true','',
    '[project]','name = "t17"','output = "o.mp4"','',
    '[source]','video = "clip.mp4"','subtitles = "clip.srt"','danmaku = ""','metadata = ""','',
    '[canvas]','width = 1920','height = 1080','',
    '[strip]','max_duration = 2.0','',
    '[[segments]]','start = 0.0','end = 2.0','role = "other"','index = 1','',
    '[[segments.events]]','type = "element"','kind = "text"','time = 0.2','end = 1.5',
    'text = "钩子标题"','style = "hook"','pos = "top"','x = "10%"','y = "20%"',
    'source = "hook|120|8|255,255,255,255|26,26,26,255|900|0"','',
  ];
  if (elementsLines.length) L.push('[elements]', ...elementsLines, '');
  const toml = L.join(NL);
  writeFileSync(join(dir,'polish.toml'), toml,'utf8');
  const cfg = mod.parseToml(toml);
  if (process.env.DBG) {
    console.log('  [dbg] segments =', JSON.stringify((cfg.segments||[]).map(sg=>({start:sg.start,end:sg.end,events:(sg.events||[]).map(e=>e.type)}))));
    console.log('  [dbg] keep/processed =', cfg.processed);
    console.log('  [dbg] source =', JSON.stringify(cfg.source));
  }
  const doc = mod.translate(cfg, dir, join(dir,'p.doc.json'));
  if (process.env.DBG) console.log('  [dbg] tracks =', JSON.stringify((doc.timeline?.tracks||[]).map(t=>t.id)));
  const tracks = doc.timeline?.tracks ?? [];
  const ids = tracks.map(t=>t.id);
  const t = tracks.find(x=>x.id==='text_hook');
  if (!t) { console.log(tag, ' -> NO text_hook; tracks =', JSON.stringify(ids)); return null; }
  return t.subtitle;
}

const a = run('A 默认', []);
if (a) {
  console.log('=== A. 全默认（老工程）===');
  console.log('  new keys present:', JSON.stringify(NEW.filter(k=>k in a)));
}
const b = run('B 全开', ['scale_in_from = 0.7','scale_in_peak = 1.15','scale_in_ms = 180','shake_px = 14.0','shake_hz = 18.0','plate_color = [0,0,0,140]','plate_pad_px = 12.0']);
if (b) {
  console.log('=== B. 全开 ===');
  for (const k of NEW) console.log('  ', k, '=', JSON.stringify(b[k]));
}
const c = run('C 只 shake_px', ['shake_px = 14.0']);
if (c) {
  console.log('=== C. 只配 shake_px、不开 typewriter ===');
  console.log('  shake_px =', JSON.stringify(c.shake_px), '| new keys:', JSON.stringify(NEW.filter(k=>k in c)));
}
