#!/usr/bin/env python3
# -*- coding: utf-8 -*-
"""subtag_komori_cn.py — 中文版 catalog 打标（7 大类 → 细分 tag）。"""
import json
from collections import defaultdict

CATEGORIES: dict[str, dict[str, list[tuple[str, str]]]] = {
    "① 主播情绪反应": {
        "爆笑搞笑": [("anime01", "噱头·动画片风格(异怪的声音)")],
        "惊吓事件": [("horror02", "事件·事故·灾难")],
        "精神攻击": [("horror01", "精神攻击")],
        "异世界恐怖": [("horror01", "异世界·地狱")],
        "恐怖氛围": [("horror01", "表示心理的声音")],
        "未知恐怖": [("horror02", "不明的声音?"), ("horror02", "他")],
        "胜负强调": [("game01", "能量提升‧減少"), ("game01", "猫按钮")],
        "打击格斗": [("attack01", "打的·踢"), ("sports01", "拳击")],
    },
    "② 游戏战斗": {
        "爆炸": [("arms01", "爆炸·破坏"), ("arms01", "游戏的爆炸声")],
        "枪械": [("arms01", "枪支·导弹等")],
        "激光射击": [("arms01", "激光·射击游戏用")],
        "挥砍斩击": [("attack01", "风切·空挥"), ("jidaigeki01", "剑·刀"), ("jidaigeki01", "切的·刺")],
        "受伤击倒": [("attack01", "损坏"), ("attack01", "他")],
        "魔法": [("magic01", "魔术")],
        "幻想": [("magic01", "幻想")],
        "怪物咆哮": [("monster01", "龙·魔兽·怪物"), ("monster01", "恶魔声"),
                     ("horror02", "僵尸·怪物·用餐")],
        "游戏系统音": [("game01", "按钮"), ("game01", "正确的解答·废除"),
                       ("game01", "拣硬币"), ("game01", "跳跃")],
        "SF空想": [("sf01", "SF･空想机械"), ("attack01", "科幻小说")],
        "机器人宇宙": [("sf01", "机器人･大型机械"), ("sf01", "宇宙")],
        "破坏倒塌": [("daily01", "滚转･坏"), ("enviroment02", "拆除工作"),
                     ("enviroment02", "工地")],
    },
    "③ 演出节目": {
        "戏剧转场": [("anime01", "场面转换·开始·结束")],
        "乐器演奏": [("playing01", "乐器")],
        "演奏会": [("event01", "演奏会")],
        "烟花游园": [("event01", "烟火"), ("event01", "游乐园")],
        "球场竞技": [("event01", "职业棒球"), ("sports01", "排球"), ("sports01", "篮球"),
                     ("sports01", "体育")],
        "宴会": [("event01", "宴会"), ("playing01", "桌面游戏")],
        "祭祀活动": [("event01", "祭祀·活动"), ("event01", "相扑")],
        "和风时代剧": [("jidaigeki01", "各种"), ("jidaigeki01", "宗教·礼仪"),
                       ("jidaigeki01", "勇士的吶喊")],
        "体育学校": [("sports01", "剑术·柔道"), ("sports01", "学校关联")],
    },
    "④ 生活日常": {
        "敲门推门": [("knocking01", "ドア・ふすま")],
        "敲击": [("knocking01", "壁・柱・床・鉄棒")],
        "房门开合": [("openclose01", "门·房间·进入")],
        "窗帘开合": [("openclose01", "窗·窗帘·快门等")],
        "玄关阳台": [("daily01", "敲·门·门口")],
        "柜子抽屉": [("openclose01", "橱·桌子·小瓶")],
        "开合物品": [("openclose01", "工具·机器")],
        "电话": [("electric01", "电话·手机")],
        "电器": [("electric01", "电气化产品·电灯"), ("electric01", "PC·外部设备·照相机")],
        "清洁遥控": [("electric01", "卫生·扫除")],
        "PC": [("pc01", "PC・マウス・キーボード"), ("pc01", "プリンタ・スキャナ"),
               ("pc01", "カメラ")],
        "脚步": [("human01", "走"), ("human01", "跑")],
        "人体动作": [("human01", "人的身体")],
        "人声效果": [("human01", "女声"), ("human01", "男声")],
        "工具挥动": [("daily01", "工具"), ("daily01", "挥")],
        "杂货": [("putting01", "杂物")],
        "放置拿起": [("putting01", "放置·举起")],
        "文具": [("daily02", "文具")],
        "房间杂物": [("daily02", "房间中的这个那个地")],
        "道具擦拭": [("daily02", "道具"), ("daily02", "擦"), ("daily02", "伞")],
        "扫除": [("daily01", "扫除")],
    },
    "⑤ 饮食水声": {
        "吃饭": [("eating01", "食物"), ("eating01", "在食用前")],
        "喝水": [("eating01", "喝"), ("eating01", "饮料")],
        "切菜": [("cooking01", "烹饪")],
        "煎炒炖煮": [("cooking01", "烹饪（烘烤・煎炸・炖煮）")],
        "其他烹饪": [("cooking01", "其他烹饪"), ("cooking01", "蛋")],
        "厨房开合": [("cooking01", "厨房·橱·桌子")],
        "厨房水洗": [("cooking01", "打扫干净")],
        "食器": [("putting01", "餐具")],
        "浴室厕所": [("daily01", "浴室·洗脸池·厕所"), ("wetarea01", "トイレ"),
                     ("wetarea01", "風呂")],
        "洗面台": [("wetarea01", "洗面台")],
        "厨房水声": [("wetarea01", "台所")],
    },
    "⑥ 环境氛围": {
        "店铺设施": [("enviroment01", "店全般·设备")],
        "都市日常": [("enviroment01", "都市的一日"), ("enviroment01", "季节")],
        "车站周边": [("tokyo01", "新宿车站"), ("tokyo01", "新宿商业区"), ("tokyo01", "东京车站"),
                     ("tokyo01", "浅草･上野动物园"), ("tokyo01", "他")],
        "美国街头": [("us01", "曼哈顿"), ("us01", "他")],
        "法国街头": [("french01", "城市"), ("french01", "拥挤･对话")],
        "乡村田野": [("french01", "农村"), ("french01", "乡村･自然")],
        "酒店人群": [("us01", "酒店")],
        "鸟": [("animals01", "鸟")],
        "猫狗": [("animals01", "狗"), ("animals01", "小狗(马尔済斯)"),
                 ("animals01", "猫"), ("animals01", "其他动物")],
        "雨雷": [("nature01", "雨·暴风雨·雷")],
        "风": [("nature01", "风")],
        "四季自然": [("nature01", "秋天"), ("nature01", "梅雨·夏天")],
        "海港": [("nature01", "海"), ("enviroment01", "港")],
        "其他自然": [("nature01", "他")],
        "工地": [("enviroment02", "室内建筑·裝修"), ("enviroment02", "河流建设"),
                 ("enviroment02", "设备")],
        "警报": [("enviroment01", "急救·消防"), ("us01", "警报系")],
        "环境噪音": [("noise01", "")],
    },
    "⑦ 交通出行": {
        "铁路轨道": [("transfer01", "JapanRailway区域内·新干线"), ("transfer01", "私营铁路电车·首页"),
                     ("transfer01", "线路"), ("french01", "高速列车(TGV)")],
        "地铁": [("transfer01", "地铁"), ("us01", "地铁･城铁"), ("french01", "巴黎的地铁")],
        "机场航站": [("transfer01", "飞机·机场"), ("us01", "在机场･飞机上"),
                     ("tokyo01", "成田机场（千叶）･火车"), ("french01", "机场")],
        "车辆行驶": [("enviroment02", "车·卡车·摩托车"), ("car01", "汽车行驶")],
        "车内设备": [("car01", "汽车设备"), ("car01", "汽车内饰"), ("car01", "车门")],
        "巴士自行车": [("transfer01", "巴士"), ("transfer01", "自行车"), ("transfer01", "他")],
    },
}

_SUBCAT_INDEX = {}
for cat, subtags in CATEGORIES.items():
    for subtag, pairs in subtags.items():
        for pair in pairs:
            if pair in _SUBCAT_INDEX:
                raise SystemExit(f"重复映射: {pair}")
            _SUBCAT_INDEX[pair] = (cat, subtag)

def main():
    cat = json.load(open("experiments/komori_cn_full.json", encoding="utf-8"))
    missing = set()
    for e in cat:
        key = (e["page"], e["subcat"])
        if key in _SUBCAT_INDEX:
            e["cat"], e["tags"] = _SUBCAT_INDEX[key][0], [_SUBCAT_INDEX[key][1]]
        else:
            missing.add(key)
            e["cat"], e["tags"] = "", []
    if missing:
        print("⚠ 未映射:")
        for m in sorted(missing):
            print(f"  {m}")
        return
    out = "experiments/komori_cn_tagged.json"
    with open(out, "w", encoding="utf-8") as f:
        json.dump(cat, f, ensure_ascii=False, indent=1)
    print(f"总条目: {len(cat)}")
    for cname, subtags in CATEGORIES.items():
        n_items = sum(1 for e in cat if e["cat"] == cname)
        print(f"{cname} — {len(subtags)} tag / {n_items} 条目")
    print(f"saved: {out}")

if __name__ == "__main__":
    main()
