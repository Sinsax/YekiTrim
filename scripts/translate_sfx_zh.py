#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
translate_sfx_zh.py — 为无官方中文的 catalog 条目生成中文声音释义（LLM 批量翻译）。
输入: experiments/komori_raw.json (含部分 label_zh 官方中文)
输出: experiments/komori_raw_labeled.json (全量 label_zh)
"""
import json, re, sys, time, urllib.request

CFG = "target/debug/.vtrim/llm.toml"
try:
    cfg = {}
    for line in open(CFG, encoding="utf-8"):
        line = line.strip()
        if "=" in line and not line.startswith("#"):
            k, v = line.split("=", 1)
            cfg[k.strip()] = v.strip().strip('"')
except FileNotFoundError:
    sys.exit("缺少 LLM 配置: " + CFG)

BASE = cfg.get("base_url", "").rstrip("/") + "/chat/completions"
KEY = cfg.get("api_key", "")
MODEL = cfg.get("model", "")

def chat(system, user):
    body = {
        "model": MODEL,
        "messages": [
            {"role": "system", "content": system},
            {"role": "user", "content": user},
        ],
        "temperature": 0.1,
        "max_tokens": 8000,
    }
    req = urllib.request.Request(BASE, data=json.dumps(body).encode(),
                                 headers={"Authorization": f"Bearer {KEY}",
                                          "Content-Type": "application/json",
                                          "User-Agent": "vtrim-catalog-builder/1.0"})
    for attempt in range(3):
        try:
            with urllib.request.urlopen(req, timeout=120) as r:
                resp = json.loads(r.read().decode())
            return resp["choices"][0]["message"]["content"]
        except Exception as e:
            print(f"  retry {attempt+1}: {e}")
            time.sleep(3)
    return ""

SYSTEM = """你是音效本地化助手。为每个音效用中文描述"它听起来像什么"（4~12 字，描述声音内容/用途，不是翻译日文标题）。
规则:
- 描述声音本身或使用场景, 如: 無茶苦茶→搞笑胡闹音, 一休さん？→一休主题搞笑音, 軽打１→轻拳打击声
- 动漫梗/文化梗音效说明其梗来源, 如: ラムちゃん？→动漫《福星小子》拉姆登场音
- 环境音描述场景, 如: 博多駅構内/長１→博多站内广播声
- 只输出 JSON 对象, 键=id, 值=中文描述"""

def main():
    raw = json.load(open("experiments/komori_raw.json", encoding="utf-8"))
    todo = [e for e in raw if not e.get("label_zh")]
    print(f"需翻译: {len(todo)} / {len(raw)}")
    BATCH = 100
    done = 0
    for i in range(0, len(todo), BATCH):
        batch = todo[i:i+BATCH]
        items = [{"id": e["id"], "jp": e["title"], "page": e["page"]} for e in batch]
        user = json.dumps(items, ensure_ascii=False)
        content = chat(SYSTEM, user)
        # 提取 JSON 对象
        m = re.search(r"\{.*\}", content, re.S)
        if not m:
            print(f"  batch {i//BATCH+1} 无 JSON, 跳过")
            continue
        try:
            mapping = json.loads(m.group(0))
        except Exception as e:
            print(f"  batch {i//BATCH+1} 解析失败: {e}")
            continue
        for e in batch:
            zh = mapping.get(e["id"], "")
            if zh:
                e["label_zh"] = zh.strip()
                done += 1
        print(f"  batch {i//BATCH+1}: +{sum(1 for e in batch if e.get('label_zh'))}")
        time.sleep(1)

    out = "experiments/komori_raw_labeled.json"
    with open(out, "w", encoding="utf-8") as f:
        json.dump(raw, f, ensure_ascii=False, indent=1)
    zh = sum(1 for e in raw if e.get("label_zh"))
    print(f"\n完成: {zh}/{len(raw)} 有中文释义 → {out}")

if __name__ == "__main__":
    main()
