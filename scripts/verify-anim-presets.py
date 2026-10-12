import io, json, os, subprocess, tempfile, shutil, re
# 逐档真跑转译器，看每档产出的底座字段
base = r'C:\Users\para\AppData\Local\Temp'
names = ['tilt_std','tilt_sm','tilt_rev','shake_std','shake_slow','shake_lg',
         'sway_std','sway_lg','sway_slow','slam_std','slam_lg','slam_squash',
         'shrink_in_std','fade_only','shrink_in_fast','pop_std','pop_soft']
src = io.open(r'F:\para\Code\YekiTrim\docs\wgpu-effect-reference\demo\polish.toml', encoding='utf-8', newline='').read()
print(f"{'档名':<16} {'底座字段'}")
for n in names:
    d = os.path.join(base, 'per-'+n)
    if os.path.isdir(d): shutil.rmtree(d)
    os.makedirs(d)
    io.open(os.path.join(d,'clip.srt'),'w',encoding='utf-8').write('')
    shutil.copy(r'F:\para\Code\YekiTrim\docs\wgpu-effect-reference\demo\clip.srt', os.path.join(d,'clip.srt'))
    body = src.rstrip() + f'''

[[segments.events]]
type = "element"
kind = "text"
time = 1.0
end = 3.0
text = "PROBE"
style = "hook"
pos = "center"
source = "hook|120|8|255,255,255,255|26,26,26,255|900|0"
x = "50%"
y = "50%"
anim = "{n}"
'''
    io.open(os.path.join(d,'polish.toml'),'w',encoding='utf-8',newline='').write(body)
    r = subprocess.run(['node', r'F:\para\Code\YekiTrim\tools\polish-to-dhampir.mjs', d,
                        '--out', os.path.join(d,'o.json')], capture_output=True, text=True, encoding='utf-8')
    p = os.path.join(d,'o.json')
    if not os.path.exists(p):
        print(f'{n:<16} ✗ 转译失败'); continue
    j = json.load(io.open(p, encoding='utf-8'))
    for t in j['timeline']['tracks']:
        if t.get('kind')!='subtitle' or 'PROBE' not in str(t.get('layers',[{}])[0].get('id','')):
            pass
        if t.get('kind')!='subtitle': continue
        # 找带 PROBE 的那条轨（id 里带档名）
        if n not in t.get('id',''): continue
        s = t.get('subtitle') or {}
        l = t['layers'][0]
        fields = []
        for k in ['scale_in_from','scale_in_peak','scale_in_ms','shake_px','shake_hz']:
            if s.get(k) is not None: fields.append(f'{k}={s[k]}')
        rot = (l.get('transform') or {}).get('rotation')
        if rot not in (None, 0): fields.append(f'rotation={rot}')
        if (l.get('transform') or {}).get('scale_y') is not None:
            fields.append(f"scale_y={(l.get('transform') or {})['scale_y']}")
        nk = len(l.get('keyframes') or [])
        if nk: fields.append(f'关键帧×{nk}')
        print(f'{n:<16} ' + (', '.join(fields) if fields else '（空 = 只淡出，走既有 opacity）'))
