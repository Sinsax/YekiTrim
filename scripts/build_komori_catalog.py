#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
build_komori_catalog.py — 爬取小森平(効果音ラボ)全站音效目录，生成全量 catalog。

用法:
  python scripts/build_komori_catalog.py                # 只抓页面生成 catalog
  python scripts/build_komori_catalog.py --download     # 同时下载全部音频（谨慎，~200MB）

输出:
  experiments/komori_full_catalog.json  全量条目 {id, file, page, subcat, title, url}
  控制台打印分类统计

注意:
  - 站方要求: 免费可商用，但禁止直接转载音效 URL —— 下载到本地缓存使用
  - 抓取节流 1s/页，请勿加速
"""
import json
import re
import sys
import time
import urllib.request
import ssl

BASE = "https://taira-komori.net"
UA = {"User-Agent": "Mozilla/5.0 (vtrim-catalog-builder; contact: local tool)"}
CTX = ssl.create_default_context()

# 页面清单: (页面, 日文分类名) — 来自 experiments/komori_categories.json
CATEGORIES = [
    ("daily01.html", "生活動作1"), ("daily02.html", "生活動作2"),
    ("game01.html", "ゲーム・ボタン音"), ("anime01.html", "ゲーム・アニメ調"),
    ("nature01.html", "自然・季節・昆虫"), ("sports01.html", "スポーツ・学校"),
    ("knocking01.html", "ノック・叩く"), ("magic01.html", "魔法・ファンタジー"),
    ("event01.html", "イベント・祭り"), ("wetarea01.html", "水回り"),
    ("sf01.html", "SF・マシン"), ("enviroment01.html", "街環境(日常)"),
    ("putting01.html", "置く・持ち上げる"), ("jidaigeki01.html", "時代劇"),
    ("enviroment02.html", "街環境(騒音)"), ("electric01.html", "生活家電"),
    ("attack01.html", "打撃・攻撃"), ("transfer01.html", "乗り物"),
    ("pc01.html", "パソコン"), ("arms01.html", "兵器・爆発"),
    ("car01.html", "自動車"), ("cooking01.html", "料理・台所"),
    ("horror02.html", "ホラー(生音)"), ("tokyo01.html", "東京"),
    ("eating01.html", "食事"), ("horror01.html", "ホラー(心理)"),
    ("us01.html", "アメリカ"), ("openclose01.html", "開閉"),
    ("monster01.html", "怪物・竜"), ("french01.html", "フランス"),
    ("animals01.html", "動物"), ("playing01.html", "楽器・遊び"),
    ("human01.html", "人間・足音"), ("noise01.html", "ノイズ"),
]


def fetch(url: str) -> bytes:
    req = urllib.request.Request(url, headers=UA)
    with urllib.request.urlopen(req, timeout=30, context=CTX) as r:
        return r.read()


def parse_page(page: str) -> list[dict]:
    """解析一个分类页 → 条目列表 [{id, file, subcat, title}]"""
    raw = fetch(f"{BASE}/{page}")
    html = raw.decode("shift_jis", errors="replace")

    # 子分类: <FONT size="4">殴る・蹴る</FONT>，记录位置，条目归属“之前最近”的子分类
    subcat_marks = []
    for m in re.finditer(r'<FONT\s+size="?4"?>(.*?)</FONT>', html, re.S | re.I):
        text = re.sub(r"<[^>]+>", "", m.group(1)).strip()
        if text:
            subcat_marks.append((m.start(), text))

    entries = []
    audio_re = re.compile(r'<audio[^>]*src="(sound/[a-z0-9_]+/[^"\']+\.mp3)"', re.I)
    for m in audio_re.finditer(html):
        src = m.group(1)
        fname = src.rsplit("/", 1)[-1]
        stem = re.sub(r"\.mp3$", "", fname, flags=re.I)
        if not stem:
            continue
        # 子分类 = 之前最近的 FONT size=4
        subcat = ""
        for pos, text in subcat_marks:
            if pos < m.start():
                subcat = text
            else:
                break
        # 标题: ① audio 前的 DIV align=right 文本 (attack01 模式)
        #        ② audio 后的相邻文本节点 (game01 模式: </audio></DIV></TD><TD>xxx <A)
        title = ""
        before = html[max(0, m.start() - 600):m.start()]
        divs = re.findall(r'<DIV[^>]*align="?right"?[^>]*>(.*?)</DIV>', before, re.S | re.I)
        if divs:
            title = re.sub(r"<[^>]+>", "", divs[-1]).strip()
        if not title:
            after = html[m.end():m.end() + 300]
            tm = re.search(r'</audio>\s*</DIV>\s*</TD>\s*<TD[^>]*>\s*([^<]{1,60}?)\s*<A ', after, re.S | re.I)
            if tm:
                title = tm.group(1).strip()
        entries.append({
            "id": stem,
            "file": fname,
            "page": page.replace(".html", ""),
            "subcat": subcat,
            "title": title,
        })
    return entries


def group_variants(entries: list[dict]) -> list[dict]:
    """变体归组: 去掉尾部数字+可选字母的 id 视为同组 (如 knocking_an_iron_door1/2/3)。
    单成员组保持原样。输出: {id(组key), label, page, subcat, variants[]}"""
    def key_of(id_: str) -> str:
        m = re.match(r"^(.*?)(\d+[a-z]?)$", id_, re.I)
        if m and len(m.group(1)) >= 3:
            return m.group(1)
        return id_

    buckets: dict[str, list[dict]] = {}
    for e in entries:
        buckets.setdefault(key_of(e["id"]), []).append(e)

    out = []
    for k, members in buckets.items():
        # 组内按文件名排序（数字升序），首条作为 label 来源
        members.sort(key=lambda e: e["id"])
        out.append({
            "id": k,
            "label": members[0]["title"],
            "page": members[0]["page"],
            "subcat": members[0]["subcat"],
            "variants": [e["file"] for e in members],
        })
    return out


def main() -> None:
    download = "--download" in sys.argv
    all_entries: list[dict] = []
    for page, cat in CATEGORIES:
        try:
            entries = parse_page(page)
            all_entries.extend(entries)
            print(f"  {page:22s} {cat:14s} {len(entries):4d} 个")
        except Exception as e:
            print(f"  {page:22s} 失败: {e}")
        time.sleep(1.0)

    # 去重（noise01 等页面可能重复引用）
    seen, uniq = set(), []
    for e in all_entries:
        if e["id"] not in seen:
            seen.add(e["id"])
            uniq.append(e)

    # 变体归组
    grouped = group_variants(uniq)
    print(f"\n原始条目: {len(uniq)} → 归组后: {len(grouped)} (缩减 {100*(len(uniq)-len(grouped))//max(len(uniq),1)}%)")

    # 原始条目 (按页面顺序, 供中文释义按序号匹配)
    raw_out = "experiments/komori_raw.json"
    with open(raw_out, "w", encoding="utf-8") as f:
        json.dump(uniq, f, ensure_ascii=False, indent=1)
    print(f"raw: {raw_out} ({len(uniq)})")

    out = "experiments/komori_full_catalog.json"
    with open(out, "w", encoding="utf-8") as f:
        json.dump(grouped, f, ensure_ascii=False, indent=1)
    print(f"catalog 已保存: {out}")

    if download:
        import os
        dest = ".yekitrim/sfx-komori"
        os.makedirs(dest, exist_ok=True)
        print(f"下载到 {dest}/ ...")
        ok, fail = 0, 0
        for e in grouped:
            for v in e["variants"]:
                path = os.path.join(dest, e["page"], v)
                os.makedirs(os.path.dirname(path), exist_ok=True)
                if os.path.exists(path):
                    ok += 1
                    continue
                try:
                    data = fetch(f"{BASE}/sound/{e['page']}/{v}")
                    with open(path, "wb") as f:
                        f.write(data)
                    ok += 1
                except Exception as ex:
                    fail += 1
                    print(f"  ✗ {e['id']}: {ex}")
                time.sleep(0.3)
        print(f"下载完成: {ok} OK, {fail} 失败")


if __name__ == "__main__":
    main()
