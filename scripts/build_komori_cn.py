#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
build_komori_cn.py — 用小森平中文版页面重建 catalog（中文标题 + sound_os2 URL）。
缺失中文版的页面(knocking01/pc01)用日文版条目 + sound_os2 URL 兜底。
输出: experiments/komori_cn_full.json（归组后, label=中文）
"""
import json, re, time, urllib.request, ssl

BASE = "https://taira-komori.net"
UA = {"User-Agent": "Mozilla/5.0 (vtrim-catalog-builder)"}
CTX = ssl.create_default_context()

# 中文版页面 (page, 页面, 中文名)
CN_PAGES = [
    ("daily01", "daily01cn.html", "生活动作1"), ("daily02", "daily02cn.html", "生活动作2"),
    ("putting01", "putting01cn.html", "放置·举起"), ("electric01", "electric01cn.html", "家用设备"),
    ("cooking01", "cooking01cn.html", "烹饪·厨房"), ("eating01", "eating01cn.html", "饭菜·吃饭"),
    ("openclose01", "openclose01cn.html", "开关音"), ("nature01", "nature01cn.html", "自然·季节"),
    ("animals01", "animals01cn.html", "动物"), ("human01", "human01cn.html", "人·脚步声"),
    ("sports01", "sports01cn.html", "体育·学校"), ("event01", "event01cn.html", "活动·宴会"),
    ("game01", "game01cn.html", "游戏·按钮"), ("anime01", "anime01cn.html", "游戏·动画片"),
    ("magic01", "magic01cn.html", "魔术·幻想"), ("sf01", "sf01cn.html", "SF·空想机械"),
    ("jidaigeki01", "jidaigeki01cn.html", "武侠·古装剧"), ("attack01", "attack01cn.html", "格斗·打击"),
    ("arms01", "arms01cn.html", "兵器·爆炸"), ("horror01", "horror01cn.html", "恐怖·心理"),
    ("horror02", "horror02cn.html", "恐怖·现实"), ("monster01", "monster01cn.html", "龙·魔兽"),
    ("noise01", "noise01cn.html", "噪音·空调"), ("playing01", "playing01cn.html", "乐器·玩具"),
    ("enviroment01", "enviroment01cn.html", "街环境1"), ("enviroment02", "enviroment02cn.html", "街环境2"),
    ("car01", "car01cn.html", "汽车"), ("transfer01", "transfer01cn.html", "交通工具"),
    ("tokyo01", "tokyo01cn.html", "东京"), ("us01", "us01cn.html", "美国纽约"),
    ("french01", "french01cn.html", "法国"),
]
# 无中文版的页面（用日文版兜底）
JP_FALLBACK = [
    ("knocking01", "knocking01.html", "敲击"), ("pc01", "pc01.html", "电脑"),
    ("wetarea01", "wetarea01.html", "水声"),
]


def fetch(url):
    req = urllib.request.Request(url, headers=UA)
    with urllib.request.urlopen(req, timeout=30, context=CTX) as r:
        return r.read()


def parse_page(page, is_cn):
    """解析页面 → 条目 [{id, file, page, subcat, label}]（按出现顺序）"""
    if is_cn:
        html = fetch(f"{BASE}/{page}cn.html").decode("utf-8", errors="replace")
        src_pat = r'sound_os2/([a-z0-9_]+/[^"\']+\.mp3)'
    else:
        html = fetch(f"{BASE}/{page}.html").decode("shift_jis", errors="replace")
        src_pat = r'sound/([a-z0-9_]+/[^"\']+\.mp3)'

    subcat_marks = []
    for m in re.finditer(r'<FONT\s+size="?4"?>(.*?)</FONT>', html, re.S | re.I):
        text = re.sub(r"<[^>]+>", "", m.group(1)).strip()
        if text:
            subcat_marks.append((m.start(), text))

    entries = []
    for m in re.finditer(r'<audio[^>]*src="' + src_pat + '"', html, re.I):
        subdir, fname = m.group(1).split("/", 1)
        stem = re.sub(r"\.mp3$", "", fname, flags=re.I)
        if not stem:
            continue
        subcat = ""
        for pos, text in subcat_marks:
            if pos < m.start():
                subcat = text
            else:
                break
        before = html[max(0, m.start() - 400):m.start()]
        divs = re.findall(r'<DIV[^>]*align="?right"?[^>]*>(.*?)</DIV>', before, re.S | re.I)
        title = re.sub(r"<[^>]+>", "", divs[-1]).strip() if divs else ""
        entries.append({"id": stem, "file": fname, "page": page, "subcat": subcat, "label": title})
    return entries


def group_variants(entries):
    """变体归组: 去尾部数字+可选字母 → 组"""
    def key_of(id_):
        m = re.match(r"^(.*?)(\d+[a-z]?)$", id_, re.I)
        if m and len(m.group(1)) >= 3:
            return m.group(1)
        return id_
    buckets = {}
    for e in entries:
        buckets.setdefault(key_of(e["id"]), []).append(e)
    out = []
    for k, members in buckets.items():
        members.sort(key=lambda e: e["id"])
        out.append({
            "id": k,
            "label": members[0]["label"],
            "page": members[0]["page"],
            "subcat": members[0]["subcat"],
            "variants": [e["file"] for e in members],
        })
    return out


def main():
    all_entries = []
    for page, fname, _ in CN_PAGES:
        try:
            entries = parse_page(page, True)
            all_entries.extend(entries)
            print(f"  {page:16s} cn {len(entries)}")
        except Exception as e:
            print(f"  {page:16s} cn 失败: {e}")
        time.sleep(0.8)
    for page, fname, _ in JP_FALLBACK:
        try:
            entries = parse_page(page, False)
            all_entries.extend(entries)
            print(f"  {page:16s} jp {len(entries)}")
        except Exception as e:
            print(f"  {page:16s} jp 失败: {e}")
        time.sleep(0.8)

    # 去重
    seen, uniq = set(), []
    for e in all_entries:
        if e["id"] not in seen:
            seen.add(e["id"])
            uniq.append(e)
    print(f"原始条目: {len(uniq)}")

    # 归组
    grouped = group_variants(uniq)
    print(f"归组后: {len(grouped)}")

    # 原始 + 归组都保存
    with open("experiments/komori_cn_raw.json", "w", encoding="utf-8") as f:
        json.dump(uniq, f, ensure_ascii=False, indent=1)
    with open("experiments/komori_cn_full.json", "w", encoding="utf-8") as f:
        json.dump(grouped, f, ensure_ascii=False, indent=1)
    print("saved: komori_cn_raw.json / komori_cn_full.json")


if __name__ == "__main__":
    main()
