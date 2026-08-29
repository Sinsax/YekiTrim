#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""
subtag_komori_catalog.py — 二级分类：大分类(7) → 细分 tag(~90)。

按直播切片场景组织大类: 主播情绪/游戏战斗/演出节目/生活日常/饮食水声/环境氛围/交通出行。
基于页面子分类 (page, subcat) 映射。输出:
  experiments/komori_subtagged_catalog.json  {id, label, page, subcat, variants, cat, tags[]}
"""
import json
from collections import Counter

CATEGORIES: dict[str, dict[str, list[tuple[str, str]]]] = {
    "① 主播情绪反应": {
        "爆笑搞笑": [("anime01", "ギャグ・アニメ調（変な音）")],
        "惊吓事件": [("horror02", "事件")],
        "精神攻击": [("horror01", "精神攻撃")],
        "异世界恐怖": [("horror01", "異世界・地獄")],
        "恐怖氛围": [("horror01", "心理を表す音")],
        "未知恐怖": [("horror02", "正体不明？"), ("horror02", "その他")],
        "打击格斗": [("attack01", "殴る・蹴る"), ("sports01", "ボクシング・格闘技")],
        "胜负强调": [("game01", "敗北・負け"), ("game01", "パワーアップ・ダウン"), ("game01", "猫っぽいボタン")],
    },
    "② 游戏战斗": {
        "爆炸": [("arms01", "爆発・破壊"), ("arms01", "ゲーム向け爆発音")],
        "枪械": [("arms01", "銃・ミサイルなど")],
        "激光射击": [("arms01", "レーザー・シューティングゲーム用")],
        "挥砍斩击": [("attack01", "風切り・素振り"), ("jidaigeki01", "剣・刀の戦い"), ("jidaigeki01", "斬る・刺す")],
        "魔法": [("magic01", "魔法")],
        "幻想": [("magic01", "ファンタジー")],
        "怪物咆哮": [("monster01", "ドラゴン・魔獣・怪物"), ("monster01", "デビルボイス"), ("monster01", "ゾンビ・怪物・食事")],
        "破坏倒塌": [("horror02", "事故・災害"), ("daily01", "転がる・割れる・落とす"), ("daily01", "潰す"), ("enviroment02", "解体工事"), ("enviroment02", "建築工事")],
        "SF空想": [("sf01", "SF・空想機械")],
        "机器人宇宙": [("sf01", "ロボット・大型機械"), ("sf01", "電気音"), ("sf01", "宇宙")],
        "游戏系统音": [("game01", "ボタン"), ("game01", "正解・キャンセル"), ("game01", "ジャンプ"), ("game01", "コインを拾う")],
        "受伤击倒": [("attack01", "ダメージ"), ("attack01", "その他"), ("attack01", "SF系")],
    },
    "③ 演出节目": {
        "戏剧转场": [("anime01", "場面転換・オープニング・エンディング")],
        "乐器演奏": [("playing01", "楽器")],
        "演奏会": [("event01", "演奏会")],
        "球场竞技": [("event01", "プロ野球"), ("sports01", "バスケットボール"), ("sports01", "バレーボール"), ("sports01", "スポーツいろいろ")],
        "宴会": [("event01", "宴会"), ("playing01", "テーブルゲーム")],
        "烟花游园": [("event01", "花火"), ("event01", "遊園地")],
        "祭祀活动": [("event01", "福岡のお祭り・イベント"), ("event01", "大相撲")],
        "和风时代剧": [("jidaigeki01", "いろいろ"), ("jidaigeki01", "宗教・儀礼"), ("jidaigeki01", "掛け声"), ("jidaigeki01", "刑罰")],
        "体育学校": [("sports01", "剣道・柔道"), ("sports01", "学校")],
    },
    "④ 生活日常": {
        "敲门推门": [("knocking01", "ドア・ふすま")],
        "房门开合": [("openclose01", "ドア・部屋・押入れ")],
        "窗帘开合": [("openclose01", "窓・カーテン・シャッター")],
        "玄关阳台": [("daily01", "扉・玄関・ベランダなど")],
        "柜子抽屉": [("openclose01", "戸棚・机")],
        "开合物品": [("openclose01", "道具・雑貨など")],
        "电话": [("electric01", "電話・携帯")],
        "电器": [("electric01", "電化製品・電灯・スイッチ")],
        "清洁遥控": [("electric01", "リモコン・電池"), ("electric01", "衛生・清掃")],
        "PC": [("pc01", "PC・マウス・キーボード"), ("pc01", "プリンタ・スキャナ"), ("pc01", "カメラ")],
        "脚步": [("human01", "歩く"), ("human01", "走る")],
        "人体动作": [("human01", "人の身体")],
        "人声效果": [("human01", "男性の音"), ("human01", "女性の音")],
        "工具挥动": [("daily01", "工具"), ("daily01", "振る")],
        "杂货": [("putting01", "雑貨類")],
        "放置拿起": [("putting01", "置く・持ち上げるのセット")],
        "文具": [("daily02", "文房具")],
        "房间杂物": [("daily02", "部屋の中のいろいろ")],
        "道具擦拭": [("daily02", "道具"), ("daily02", "擦る（こする）"), ("daily02", "傘")],
        "扫除": [("daily01", "掃除")],
        "敲击": [("knocking01", "壁・柱・床・鉄棒")],
    },
    "⑤ 饮食水声": {
        "吃饭": [("eating01", "食べる前の動作"), ("eating01", "食べる")],
        "喝水": [("eating01", "飲む前の動作"), ("eating01", "飲む")],
        "切菜": [("cooking01", "料理（切る・削る）")],
        "煎炒炖煮": [("cooking01", "料理（焼く・炒める・煮る）")],
        "其他烹饪": [("cooking01", "その他調理")],
        "厨房开合": [("cooking01", "台所・戸棚の開閉")],
        "食器": [("putting01", "食器類")],
        "厨房水洗": [("cooking01", "後片付け"), ("wetarea01", "台所")],
        "洗面台": [("wetarea01", "洗面台")],
        "浴室厕所": [("wetarea01", "風呂"), ("wetarea01", "トイレ")],
    },
    "⑥ 环境氛围": {
        "店铺设施": [("enviroment01", "お店全般・建物設備")],
        "车站周边": [("tokyo01", "新宿"), ("tokyo01", "新宿駅"), ("tokyo01", "東京駅"), ("tokyo01", "浅草寺・上野動物園"), ("tokyo01", "秋葉原"), ("tokyo01", "その他")],
        "都市日常": [("enviroment01", "都会の一日")],
        "美国街头": [("us01", "マンハッタン街"), ("us01", "その他")],
        "法国街头": [("french01", "都会の様子"), ("french01", "人混み・会話")],
        "乡村田野": [("french01", "田舎・自然"), ("french01", "農村")],
        "酒店人群": [("us01", "ホテル")],
        "鸟": [("animals01", "鳥")],
        "猫狗": [("animals01", "猫"), ("animals01", "犬"), ("animals01", "小型犬（マルチーズ）"), ("animals01", "他の動物")],
        "雨雷": [("nature01", "雨・嵐・雷")],
        "风": [("nature01", "風")],
        "四季自然": [("nature01", "梅雨・夏"), ("nature01", "秋")],
        "其他自然": [("nature01", "その他")],
        "工地": [("enviroment02", "室内工事・リフォーム"), ("enviroment02", "河川工事"), ("enviroment02", "設備など")],
        "警报": [("enviroment01", "救急・消防"), ("us01", "サイレン系")],
        "环境噪音": [("noise01", "効果音検索"), ("enviroment01", "季節")],
        "海港": [("nature01", "海"), ("enviroment01", "港")],
    },
    "⑦ 交通出行": {
        "铁路轨道": [("transfer01", "JR構内・新幹線"), ("transfer01", "私鉄電車・ホーム（福岡）"), ("transfer01", "線路"), ("transfer01", "その他"), ("french01", "高速列車（TGV）")],
        "地铁": [("transfer01", "地下鉄（福岡）"), ("us01", "駅・地下鉄"), ("french01", "パリの地下鉄")],
        "车辆行驶": [("enviroment02", "車・トラック・バイク"), ("car01", "走行")],
        "车内设备": [("car01", "自動車機器"), ("car01", "自動車内部"), ("car01", "ドア開閉")],
        "巴士自行车": [("transfer01", "自転車"), ("transfer01", "バス（福岡）")],
        "机场航站": [("tokyo01", "羽田空港・成田空港（千葉）など"),
                    ("transfer01", "飛行機・空港"), ("us01", "空港・飛行機内"),
                    ("french01", "空港")],
    },
}

# 反向索引: (page, subcat) → (cat, subtag)
_SUBCAT_INDEX: dict[tuple[str, str], tuple[str, str]] = {}
for cat, subtags in CATEGORIES.items():
    for subtag, pairs in subtags.items():
        for pair in pairs:
            if pair in _SUBCAT_INDEX:
                raise SystemExit(f"重复映射: {pair}")
            _SUBCAT_INDEX[pair] = (cat, subtag)


def main() -> None:
    cat = json.load(open("experiments/komori_full_catalog.json", encoding="utf-8"))
    missing: set[tuple[str, str]] = set()
    for e in cat:
        key = (e["page"], e["subcat"])
        if key in _SUBCAT_INDEX:
            e["cat"], e["tags"] = _SUBCAT_INDEX[key][0], [_SUBCAT_INDEX[key][1]]
        else:
            missing.add(key)
            e["cat"], e["tags"] = "", []

    if missing:
        print("⚠ 未映射的子分类:")
        for m in sorted(missing):
            print(f"  {m}")
        return

    out = "experiments/komori_subtagged_catalog.json"
    with open(out, "w", encoding="utf-8") as f:
        json.dump(cat, f, ensure_ascii=False, indent=1)

    print(f"总条目: {len(cat)}")
    for cname, subtags in CATEGORIES.items():
        n_sub = len(subtags)
        n_items = sum(1 for e in cat if e["cat"] == cname)
        print(f"{cname} - {n_sub} 个细分 tag, {n_items} 条目")
        for s in subtags:
            cnt = sum(1 for e in cat if s in e["tags"])
            print(f"    {s:8s} {cnt:4d}")
    print(f"catalog 已保存: {out}")


if __name__ == "__main__":
    main()
