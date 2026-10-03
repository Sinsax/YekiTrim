#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
fetch_komori_cn.py — 爬取小森平中文版页面(xxxcn.html)，为 catalog 补充中文释义。
中文版条目与日文版按 (page, 出现顺序) 一一对应。输出: experiments/komori_labeled_catalog.json
"""
import json, re, time, urllib.request, ssl

BASE = "https://taira-komori.net"
UA = {"User-Agent": "Mozilla/5.0 (yekitrim-catalog-builder)"}
CTX = ssl.create_default_context()

CN_PAGES = [
    "animals01cn.html", "anime01cn.html", "arms01cn.html", "attack01cn.html",
    "car01cn.html", "cooking01cn.html", "daily01cn.html", "daily02cn.html",
    "eating01cn.html", "electric01cn.html", "enviroment01cn.html",
    "enviroment02cn.html", "event01cn.html", "french01cn.html", "game01cn.html",
    "horror01cn.html", "horror02cn.html", "human01cn.html", "jidaigeki01cn.html",
    "magic01cn.html", "monster01cn.html", "nature01cn.html", "noise01cn.html",
    "openclose01cn.html", "playing01cn.html", "putting01cn.html", "sf01cn.html",
    "sports01cn.html", "tokyo01cn.html", "transfer01cn.html", "us01cn.html",
]

def fetch(url):
    req = urllib.request.Request(url, headers=UA)
    with urllib.request.urlopen(req, timeout=30, context=CTX) as r:
        return r.read()

def parse_cn_page(page):
    raw = fetch(f"{BASE}/{page}")
    html = raw.decode("utf-8", errors="replace")
    subcat_marks = []
    for m in re.finditer(r'<FONT\s+size="?4"?>(.*?)</FONT>', html, re.S | re.I):
        text = re.sub(r"<[^>]+>", "", m.group(1)).strip()
        if text:
            subcat_marks.append((m.start(), text))
    entries = []
    for m in re.finditer(r'<audio[^>]*src="(sound_os2/[^"\']+\.mp3)"', html, re.I):
        fname = m.group(1).rsplit("/", 1)[-1]
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
        entries.append({"stem": stem, "label_zh": title, "subcat_zh": subcat})
    return entries

def main():
    cat = json.load(open("experiments/komori_raw.json", encoding="utf-8"))
    from collections import defaultdict
    by_page = defaultdict(list)
    for e in cat:
        by_page[e["page"]].append(e)

    matched = 0
    missing_pages = set()
    for cn_page in CN_PAGES:
        page = cn_page.replace("cn.html", "")
        if page not in by_page:
            missing_pages.add(page)
            continue
        try:
            entries = parse_cn_page(cn_page)
        except Exception as ex:
            print(f"  WARN {cn_page} failed: {ex}")
            continue
        page_items = by_page[page]
        if len(entries) != len(page_items):
            # 条目数不一致 → 顺序不对应, 跳过该页 (避免错位的中文释义误导)
            print(f"  SKIP {page}: cn {len(entries)} vs jp {len(page_items)} (顺序不对应)")
            continue
        for i, zh in enumerate(entries):
            page_items[i]["label_zh"] = zh["label_zh"]
            page_items[i]["subcat_zh"] = zh["subcat_zh"]
            matched += 1
        time.sleep(0.8)

    no_zh = [e for e in cat if not e.get("label_zh")]
    print(f"\nmatched: {matched} / {len(cat)}")
    print(f"no zh: {len(no_zh)} (missing pages: {sorted(missing_pages) if missing_pages else 'none'})")
    out = "experiments/komori_raw_labeled.json"
    with open(out, "w", encoding="utf-8") as f:
        json.dump(cat, f, ensure_ascii=False, indent=1)
    print(f"saved: {out}")

if __name__ == "__main__":
    main()
