#!/usr/bin/env python3
# SPDX-License-Identifier: Apache-2.0
"""Deterministically apply the Phase-1 Chinese lexicon expansion offline.

Reads traceable curated seeds from ``data_sources/lexicon_fragments/phase1/``,
resolves and verifies pinyin using empirical primary readings learned from the
repository's verified lexicons, pinned ``rust-pinyin`` character readings, and
project polyphone overrides. Promotes clean high-frequency four-character
phrases into ``lexicon/zh/life_common_4char.txt``, updates ``modern_life.tsv``
and ``seasonal_hot_terms.tsv``, and runs ``scripts/lexicon_quality.py --apply``
to refresh manifests and inventory.
"""

from __future__ import annotations

import argparse
from collections import Counter, OrderedDict, defaultdict
import json
from pathlib import Path
import re
import sys

ROOT = Path(__file__).resolve().parents[1]
if str(ROOT / "scripts") not in sys.path:
    sys.path.insert(0, str(ROOT / "scripts"))

PHASE1_DIR = ROOT / "data_sources" / "lexicon_fragments" / "phase1"
HANZI_RE = re.compile(r"^[\u3400-\u4dbf\u4e00-\u9fff\uf900-\ufaff]+$")

# Conservative score tiers for default-enabled zh-ext additions so new entries
# are easily recalled without overriding core/base everyday words.
EXT_TIER_SCORES = {
    "high": 4600,
    "mid": 3600,
    "std": 2800,
}

# Domain polyphone overrides complementing data_sources/kaixin/polyphone_corrections.tsv.
# Kept concise and conflict-free because empirical lexicon statistics already
# supply the dominant modern reading for every single character.
EXTRA_POLYPHONES: dict[str, tuple[str, ...]] = {
    "干锅": ("gan", "guo"),
    "干煸": ("gan", "bian"),
    "干贝": ("gan", "bei"),
    "干蒸": ("gan", "zheng"),
    "干炒": ("gan", "chao"),
    "干拌": ("gan", "ban"),
    "干红": ("gan", "hong"),
    "干白": ("gan", "bai"),
    "干邑": ("gan", "yi"),
    "干碟": ("gan", "die"),
    "干辣椒": ("gan", "la", "jiao"),
    "干香菇": ("gan", "xiang", "gu"),
    "干海带": ("gan", "hai", "dai"),
    "干鱿鱼": ("gan", "you", "yu"),
    "干鲍": ("gan", "bao"),
    "干虾": ("gan", "xia"),
    "干酵母": ("gan", "jiao", "mu"),
    "干炸": ("gan", "zha"),
    "干煎": ("gan", "jian"),
    "干烧": ("gan", "shao"),
    "干巴菌": ("gan", "ba", "jun"),
    "干豆腐": ("gan", "dou", "fu"),
    "干饭": ("gan", "fan"),
    "干货": ("gan", "huo"),
    "骨干": ("gu", "gan"),
    "干杯": ("gan", "bei"),
    "干净": ("gan", "jing"),
    "干嘛": ("gan", "ma"),
    "干噎": ("gan", "ye"),
    "风干": ("feng", "gan"),
    "豆干": ("dou", "gan"),
    "香干": ("xiang", "gan"),
    "茶干": ("cha", "gan"),
    "熏干": ("xun", "gan"),
    "饼干": ("bing", "gan"),
    "肉干": ("rou", "gan"),
    "笋干": ("sun", "gan"),
    "梅干菜": ("mei", "gan", "cai"),
    "葡萄干": ("pu", "tao", "gan"),
    "芒果干": ("mang", "guo", "gan"),
    "草莓干": ("cao", "mei", "gan"),
    "红薯干": ("hong", "shu", "gan"),
    "地瓜干": ("di", "gua", "gan"),
    "萝卜干": ("luo", "bo", "gan"),
    "冻干": ("dong", "gan"),
    "咖喱": ("ga", "li"),
    "咖啡": ("ka", "fei"),
    "鸭血": ("ya", "xue"),
    "猪血": ("zhu", "xue"),
    "羊血": ("yang", "xue"),
    "鹅血": ("e", "xue"),
    "血旺": ("xue", "wang"),
    "血糯": ("xue", "nuo"),
    "血肠": ("xue", "chang"),
    "血豆": ("xue", "dou"),
    "血橙": ("xue", "cheng"),
    "血鸭": ("xue", "ya"),
    "血腥": ("xue", "xing"),
    "血脉": ("xue", "mai"),
    "血亏": ("xue", "kui"),
    "满血": ("man", "xue"),
    "一针见血": ("yi", "zhen", "jian", "xue"),
    "海参": ("hai", "shen"),
    "辽参": ("liao", "shen"),
    "刺参": ("ci", "shen"),
    "人参": ("ren", "shen"),
    "党参": ("dang", "shen"),
    "参鸡汤": ("shen", "ji", "tang"),
    "花旗参": ("hua", "qi", "shen"),
    "大参林": ("da", "shen", "lin"),
    "薄荷": ("bo", "he"),
    "薄底": ("bao", "di"),
    "薄盐": ("bao", "yan"),
    "薄芡": ("bao", "qian"),
    "薄脆": ("bao", "cui"),
    "肉脯": ("rou", "fu"),
    "果脯": ("guo", "fu"),
    "杏脯": ("xing", "fu"),
    "桃脯": ("tao", "fu"),
    "菜脯": ("cai", "pu"),
    "胸脯": ("xiong", "pu"),
    "扒鸡": ("pa", "ji"),
    "猪扒": ("zhu", "pa"),
    "鸡扒": ("ji", "pa"),
    "扒肘": ("pa", "zhou"),
    "腌笃鲜": ("yan", "du", "xian"),
    "生腌": ("sheng", "yan"),
    "腌制": ("yan", "zhi"),
    "腌渍": ("yan", "zi"),
    "重芝士": ("zhong", "zhi", "shi"),
    "重疾": ("zhong", "ji"),
    "重庆": ("chong", "qing"),
    "重启": ("chong", "qi"),
    "重做": ("chong", "zuo"),
    "重来": ("chong", "lai"),
    "重归": ("chong", "gui"),
    "重蹈": ("chong", "dao"),
    "重整": ("chong", "zheng"),
    "重温": ("chong", "wen"),
    "重操": ("chong", "cao"),
    "重修": ("chong", "xiu"),
    "重逢": ("chong", "feng"),
    "重聚": ("chong", "ju"),
    "重建": ("chong", "jian"),
    "重组": ("chong", "zu"),
    "重构": ("chong", "gou"),
    "重塑": ("chong", "su"),
    "重装": ("chong", "zhuang"),
    "重置": ("chong", "zhi"),
    "重排": ("chong", "pai"),
    "重算": ("chong", "suan"),
    "重发": ("chong", "fa"),
    "重试": ("chong", "shi"),
    "重连": ("chong", "lian"),
    "重写": ("chong", "xie"),
    "重审": ("chong", "shen"),
    "重播": ("chong", "bo"),
    "重申": ("chong", "shen"),
    "重现": ("chong", "xian"),
    "重拾": ("chong", "shi"),
    "重返": ("chong", "fan"),
    "重回": ("chong", "hui"),
    "重生": ("chong", "sheng"),
    "重名": ("chong", "ming"),
    "重样": ("chong", "yang"),
    "去重": ("qu", "chong"),
    "查重": ("cha", "chong"),
    "降重": ("jiang", "chong"),
    "双重": ("shuang", "chong"),
    "多重": ("duo", "chong"),
    "三重": ("san", "chong"),
    "九重": ("jiu", "chong"),
    "秘制": ("mi", "zhi"),
    "番茄": ("fan", "qie"),
    "茄子": ("qie", "zi"),
    "茄盒": ("qie", "he"),
    "杭茄": ("hang", "qie"),
    "线茄": ("xian", "qie"),
    "角色": ("jue", "se"),
    "主角": ("zhu", "jue"),
    "角逐": ("jue", "zhu"),
    "调休": ("tiao", "xiu"),
    "调料": ("tiao", "liao"),
    "调味": ("tiao", "wei"),
    "调和": ("tiao", "he"),
    "调整": ("tiao", "zheng"),
    "调薪": ("tiao", "xin"),
    "空调": ("kong", "tiao"),
    "特调": ("te", "tiao"),
    "调性": ("diao", "xing"),
    "调研": ("diao", "yan"),
    "调岗": ("diao", "gang"),
    "调动": ("diao", "dong"),
    "调度": ("diao", "du"),
    "调拨": ("diao", "bo"),
    "低调": ("di", "diao"),
    "出差": ("chu", "chai"),
    "差旅": ("chai", "lv"),
    "校对": ("jiao", "dui"),
    "校准": ("jiao", "zhun"),
    "会计": ("kuai", "ji"),
    "财会": ("cai", "kuai"),
    "会稽山": ("kuai", "ji", "shan"),
    "便宜坊": ("bian", "yi", "fang"),
    "便宜": ("pian", "yi"),
    "便当": ("bian", "dang"),
    "发卡": ("fa", "qia"),
    "绿豆": ("lv", "dou"),
    "绿茶": ("lv", "cha"),
    "绿道": ("lv", "dao"),
    "绿电": ("lv", "dian"),
    "绿植": ("lv", "zhi"),
    "绿幕": ("lv", "mu"),
    "绿色": ("lv", "se"),
    "绿水": ("lv", "shui"),
    "驴肉": ("lv", "rou"),
    "驴打滚": ("lv", "da", "gun"),
    "策略": ("ce", "lve"),
    "战略": ("zhan", "lve"),
    "攻略": ("gong", "lve"),
    "略过": ("lve", "guo"),
    "简略": ("jian", "lve"),
    "忽略": ("hu", "lve"),
    "着凉": ("zhao", "liang"),
    "着急": ("zhao", "ji"),
    "着火": ("zhao", "huo"),
    "睡着": ("shui", "zhao"),
    "找不着": ("zhao", "bu", "zhao"),
    "歪打正着": ("wai", "da", "zheng", "zhao"),
    "着重": ("zhuo", "zhong"),
    "着手": ("zhuo", "shou"),
    "着力": ("zhuo", "li"),
    "着陆": ("zhuo", "lu"),
    "穿着": ("chuan", "zhuo"),
    "执着": ("zhi", "zhuo"),
    "反省": ("fan", "xing"),
    "发人深省": ("fa", "ren", "shen", "xing"),
    "露一手": ("lou", "yi", "shou"),
    "露馅": ("lou", "xian"),
    "露脸": ("lou", "lian"),
    "宝藏": ("bao", "zang"),
    "藏族": ("zang", "zu"),
    "西藏": ("xi", "zang"),
    "藏红花": ("zang", "hong", "hua"),
    "模具": ("mu", "ju"),
    "模样": ("mu", "yang"),
    "一模一样": ("yi", "mu", "yi", "yang"),
    "像模像样": ("xiang", "mu", "xiang", "yang"),
    "有模有样": ("you", "mu", "you", "yang"),
    "大模大样": ("da", "mu", "da", "yang"),
    "行业": ("hang", "ye"),
    "银行": ("yin", "hang"),
    "开户行": ("kai", "hu", "hang"),
    "行长": ("hang", "zhang"),
    "行家": ("hang", "jia"),
    "行情": ("hang", "qing"),
    "转行": ("zhuan", "hang"),
    "内行": ("nei", "hang"),
    "外行": ("wai", "hang"),
    "还贷": ("huan", "dai"),
    "还款": ("huan", "kuan"),
    "归还": ("gui", "huan"),
    "还钱": ("huan", "qian"),
    "还清": ("huan", "qing"),
    "还给": ("huan", "gei"),
    "偿还": ("chang", "huan"),
    "返还": ("fan", "huan"),
    "退还": ("tui", "huan"),
    "长胖": ("zhang", "pang"),
    "长高": ("zhang", "gao"),
    "长大": ("zhang", "da"),
    "成长": ("cheng", "zhang"),
    "增长": ("zeng", "zhang"),
    "长见识": ("zhang", "jian", "shi"),
    "组长": ("zu", "zhang"),
    "科长": ("ke", "zhang"),
    "部长": ("bu", "zhang"),
    "处长": ("chu", "zhang"),
    "院长": ("yuan", "zhang"),
    "所长": ("suo", "zhang"),
    "厂长": ("chang", "zhang"),
    "店长": ("dian", "zhang"),
    "家长": ("jia", "zhang"),
    "董事长": ("dong", "shi", "zhang"),
    "校长": ("xiao", "zhang"),
    "粘豆包": ("nian", "dou", "bao"),
    "粘人": ("nian", "ren"),
    "粘米粉": ("zhan", "mi", "fen"),
    "不粘锅": ("bu", "zhan", "guo"),
    "粘贴": ("zhan", "tie"),
    "供给": ("gong", "ji"),
    "自给": ("zi", "ji"),
    "补给": ("bu", "ji"),
    "给排水": ("ji", "pai", "shui"),
    "率先": ("shuai", "xian"),
    "率领": ("shuai", "ling"),
    "老火靓汤": ("lao", "huo", "liang", "tang"),
    "靓汤": ("liang", "tang"),
    "朝气": ("zhao", "qi"),
    "一朝一夕": ("yi", "zhao", "yi", "xi"),
    "弹幕": ("dan", "mu"),
    "子弹": ("zi", "dan"),
    "单县": ("shan", "xian"),
    "阿胶": ("e", "jiao"),
    "东阿": ("dong", "e"),
    "阿弥陀佛": ("e", "mi", "tuo", "fo"),
    "刚正不阿": ("gang", "zheng", "bu", "e"),
    "盛饭": ("cheng", "fan"),
    "盛汤": ("cheng", "tang"),
    "盛菜": ("cheng", "cai"),
    "和面": ("huo", "mian"),
    "搅和": ("jiao", "huo"),
    "暖和": ("nuan", "huo"),
    "称职": ("chen", "zhi"),
    "对称": ("dui", "chen"),
    "称心": ("chen", "xin"),
    "相称": ("xiang", "chen"),
    "匀称": ("yun", "chen"),
    "浑身解数": ("hun", "shen", "xie", "shu"),
    "非得": ("fei", "dei"),
    "总得": ("zong", "dei"),
    "还得": ("hai", "dei"),
    "了不起": ("liao", "bu", "qi"),
    "了不得": ("liao", "bu", "de"),
    "了如指掌": ("liao", "ru", "zhi", "zhang"),
    "一目了然": ("yi", "mu", "liao", "ran"),
    "不了了之": ("bu", "liao", "liao", "zhi"),
    "一笑了之": ("yi", "xiao", "liao", "zhi"),
    "没完没了": ("mei", "wan", "mei", "liao"),
    "受不了": ("shou", "bu", "liao"),
    "少不了": ("shao", "bu", "liao"),
    "免不了": ("mian", "bu", "liao"),
    "忘不了": ("wang", "bu", "liao"),
    "去不了": ("qu", "bu", "liao"),
    "吃不了": ("chi", "bu", "liao"),
    "用不了": ("yong", "bu", "liao"),
    "大不了": ("da", "bu", "liao"),
    "了结": ("liao", "jie"),
    "了断": ("liao", "duan"),
    "直截了当": ("zhi", "jie", "liao", "dang"),
    "大夫": ("dai", "fu"),
    "好大夫": ("hao", "dai", "fu"),
    "审时度势": ("shen", "shi", "duo", "shi"),
    "以己度人": ("yi", "ji", "duo", "ren"),
    "倔强": ("jue", "jiang"),
    "参差": ("cen", "ci"),
    "闭塞": ("bi", "se"),
    "堵塞": ("du", "se"),
    "阻塞": ("zu", "se"),
    "茅塞顿开": ("mao", "se", "dun", "kai"),
    "敷衍塞责": ("fu", "yan", "se", "ze"),
    "塞翁失马": ("sai", "weng", "shi", "ma"),
    "亲家": ("qing", "jia"),
    "湖泊": ("hu", "po"),
    "血泊": ("xue", "po"),
    "刨冰": ("bao", "bing"),
    "刨丝": ("bao", "si"),
    "攒局": ("cuan", "ju"),
    "电饼铛": ("dian", "bing", "cheng"),
    "壳牌": ("qiao", "pai"),
    "地壳": ("di", "qiao"),
    "甲壳": ("jia", "qiao"),
    "金蝉脱壳": ("jin", "chan", "tuo", "qiao"),
    "水灵灵地": ("shui", "ling", "ling", "de"),
    "慢慢地": ("man", "man", "de"),
    "悄悄地": ("qiao", "qiao", "de"),
    "偷偷地": ("tou", "tou", "de"),
    "的士": ("di", "shi"),
    "的确": ("di", "que"),
    "目的": ("mu", "di"),
    "标的": ("biao", "di"),
    "蛤蜊": ("ge", "li"),
    "花蛤": ("hua", "ge"),
    "文蛤": ("wen", "ge"),
    "白蛤": ("bai", "ge"),
    "青蛤": ("qing", "ge"),
    "毛蛤": ("mao", "ge"),
    "鮰鱼": ("hui", "yu"),
    "耙耙柑": ("pa", "pa", "gan"),
    "啫啫": ("zhe", "zhe"),
    "胸口朥": ("xiong", "kou", "lao"),
    "罾蹦": ("zeng", "beng"),
    "嘎巴菜": ("ga", "ba", "cai"),
    "格拉条": ("ge", "la", "tiao"),
    "撒撇": ("sa", "pie"),
    "舂鸡脚": ("chong", "ji", "jiao"),
    "自怨自艾": ("zi", "yuan", "zi", "yi"),
    "方兴未艾": ("fang", "xing", "wei", "ai"),
    "丢三落四": ("diu", "san", "la", "si"),
    "系鞋带": ("ji", "xie", "dai"),
    "单于": ("chan", "yu"),
    "都匀": ("du", "yun"),
    "首都": ("shou", "du"),
    "成都": ("cheng", "du"),
    "都市": ("du", "shi"),
    "名都": ("ming", "du"),
    "古都": ("gu", "du"),
    "京都": ("jing", "du"),
    "魔都": ("mo", "du"),
    "帝都": ("di", "du"),
    "国都": ("guo", "du"),
    "都有": ("dou", "you"),
    "都是": ("dou", "shi"),
    "都会好": ("dou", "hui", "hao"),
    "都行": ("dou", "xing"),
    "都好": ("dou", "hao"),
    "全都": ("quan", "dou"),
    "也都": ("ye", "dou"),
    "我都": ("wo", "dou"),
    "你都": ("ni", "dou"),
    "每天都": ("mei", "tian", "dou"),
    "一点都不": ("yi", "dian", "dou", "bu"),
    "什么都": ("shen", "me", "dou"),
    "哪天都": ("na", "tian", "dou"),
    "什么时候都": ("shen", "me", "shi", "hou", "dou"),
    "心都": ("xin", "dou"),
    "头都": ("tou", "dou"),
    "都挺好": ("dou", "ting", "hao"),
    "都可以": ("dou", "ke", "yi"),
    "都没有": ("dou", "mei", "you"),
    "都过去了": ("dou", "guo", "qu", "le"),
    "都听你的": ("dou", "ting", "ni", "de"),
    "投降": ("tou", "xiang"),
    "降服": ("xiang", "fu"),
    "睡觉": ("shui", "jiao"),
    "午觉": ("wu", "jiao"),
    "懒觉": ("lan", "jiao"),
    "补觉": ("bu", "jiao"),
    "好觉": ("hao", "jiao"),
    "音乐": ("yin", "yue"),
    "乐器": ("yue", "qi"),
    "乐队": ("yue", "dui"),
    "乐曲": ("yue", "qu"),
    "乐谱": ("yue", "pu"),
    "乐章": ("yue", "zhang"),
    "乐手": ("yue", "shou"),
    "乐坛": ("yue", "tan"),
    "乐迷": ("yue", "mi"),
    "交响乐": ("jiao", "xiang", "yue"),
    "摇滚乐": ("yao", "gun", "yue"),
    "爵士乐": ("jue", "shi", "yue"),
    "民乐": ("min", "yue"),
    "配乐": ("pei", "yue"),
    "哀乐": ("ai", "yue"),
    "爱乐": ("ai", "yue"),
    "数一数二": ("shu", "yi", "shu", "er"),
    "数不清": ("shu", "bu", "qing"),
    "阿嬷": ("a", "mo"),
    "龟苓膏": ("gui", "ling", "gao"),
    "螺蛳粉": ("luo", "si", "fen"),
    "荸荠": ("bi", "qi"),
    "雪里蕻": ("xue", "li", "hong"),
    "粢饭": ("ci", "fan"),
    "糍粑": ("ci", "ba"),
    "栲栳栳": ("kao", "lao", "lao"),
    "莜面": ("you", "mian"),
    "醪糟": ("lao", "zao"),
    "甜胚子": ("tian", "pei", "zi"),
    "拨冗": ("bo", "rong"),
    "莅临": ("li", "lin"),
    "谢忱": ("xie", "chen"),
    "商祺": ("shang", "qi"),
    "大腹便便": ("da", "fu", "pian", "pian"),
    "一曝十寒": ("yi", "pu", "shi", "han"),
    "张牙舞爪": ("zhang", "ya", "wu", "zhao"),
    "差强人意": ("cha", "qiang", "ren", "yi"),
    "前仆后继": ("qian", "pu", "hou", "ji"),
    "风尘仆仆": ("feng", "chen", "pu", "pu"),
    "味同嚼蜡": ("wei", "tong", "jiao", "la"),
    "咀嚼": ("ju", "jue"),
    "哽咽": ("geng", "ye"),
    "强劲": ("qiang", "jing"),
    "一唱一和": ("yi", "chang", "yi", "he"),
    "曲高和寡": ("qu", "gao", "he", "gua"),
    "随声附和": ("sui", "sheng", "fu", "he"),
    "压轴": ("ya", "zhou"),
    "压轴戏": ("ya", "zhou", "xi"),
}


def normalize_syllable(syl: str) -> str:
    return syl.strip().lower().replace("ü", "v").replace("nue", "nve").replace("lue", "lve")


def normalize_reading(reading: str) -> str:
    return " ".join(normalize_syllable(tok) for tok in reading.split() if tok.strip())


class OfflinePinyinResolver:
    """Resolve Chinese phrases using empirical lexicon frequencies + project overrides."""

    def __init__(self, root: Path) -> None:
        self.root = root
        self.char_primary: dict[str, str] = {}
        self.char_all: dict[str, set[str]] = {}
        self.exact_phrases: dict[str, str] = {}
        self.sub_phrases: dict[str, tuple[str, ...]] = {}
        self.poly_exact: dict[str, tuple[str, ...]] = {}
        self.poly_sub: list[tuple[str, tuple[str, ...]]] = []
        self._load()

    def _load(self) -> None:
        from lexicon_quality import character_readings

        self.char_all = character_readings(self.root)

        # 1) Seed fallback readings from rust-pinyin snapshot
        readings_path = self.root / "pinyin-ime" / "data" / "pinyin_character_readings.txt"
        for line in readings_path.read_text(encoding="utf-8-sig").splitlines():
            if not line.strip() or line.startswith("#"):
                continue
            ch, codes = line.split("\t")
            parts = [normalize_syllable(c) for c in codes.split() if c.strip()]
            if parts:
                self.char_primary[ch] = parts[0]

        # 2) Learn true empirical primary reading per char from all verified lexicons!
        # This avoids alphabetically-first rare readings in rust-pinyin (e.g. 液->shi, 排->bai).
        char_votes: dict[str, Counter[str]] = defaultdict(Counter)
        best_existing: dict[str, tuple[int, str]] = {}
        for folder in ("core", "zh", "zh-ext"):
            d = self.root / "lexicon" / folder
            if not d.is_dir():
                continue
            for path in sorted(d.glob("*.txt")):
                if path.name == "single_char_common_8105.txt":
                    continue
                for line in path.read_text(encoding="utf-8-sig").splitlines():
                    if not line.strip() or line.startswith("#"):
                        continue
                    fields = line.split("\t")
                    if len(fields) < 3:
                        continue
                    phrase, reading, score_s = fields[0], normalize_reading(fields[1]), fields[2]
                    if not HANZI_RE.match(phrase):
                        continue
                    syls = reading.split()
                    if len(syls) != len(phrase):
                        continue
                    score = int(score_s)
                    prev = best_existing.get(phrase)
                    if prev is None or score > prev[0]:
                        best_existing[phrase] = (score, reading)
                    # Weight votes gently so frequent everyday words dominate
                    vote_w = 2 if folder == "zh" else 1
                    for ch, syl in zip(phrase, syls):
                        if syl in self.char_all.get(ch, set()):
                            char_votes[ch][syl] += vote_w

        for ch, counter in char_votes.items():
            self.char_primary[ch] = counter.most_common(1)[0][0]

        for phrase, (_, reading) in best_existing.items():
            self.exact_phrases[phrase] = reading
            if len(phrase) in (2, 3):
                self.sub_phrases[phrase] = tuple(reading.split())

        # 3) Apply project single-char primary overrides and exclusions
        aliases_path = self.root / "data_sources" / "kaixin" / "pronunciation_aliases.tsv"
        for line in aliases_path.read_text(encoding="utf-8-sig").splitlines():
            if not line.strip() or line.startswith("#"):
                continue
            fields = line.split("\t")
            if len(fields) >= 4 and len(fields[0]) == 1 and fields[3] == "primary":
                self.char_primary[fields[0]] = normalize_syllable(fields[1])

        excl_path = self.root / "data_sources" / "kaixin" / "pronunciation_exclusions.tsv"
        for line in excl_path.read_text(encoding="utf-8-sig").splitlines():
            if not line.strip() or line.startswith("#"):
                continue
            fields = line.split("\t")
            if len(fields) >= 2 and len(fields[0]) == 1:
                ch = fields[0]
                bad = normalize_syllable(fields[1])
                if self.char_primary.get(ch) == bad:
                    remaining = sorted(self.char_all.get(ch, set()) - {bad})
                    if remaining:
                        self.char_primary[ch] = remaining[0]

        # 4) Load project polyphone_corrections.tsv + domain polyphones
        poly_path = self.root / "data_sources" / "kaixin" / "polyphone_corrections.tsv"
        sub_map: dict[str, tuple[str, ...]] = dict(EXTRA_POLYPHONES)
        for line in poly_path.read_text(encoding="utf-8-sig").splitlines():
            if not line.strip() or line.startswith("#"):
                continue
            fields = [f.strip() for f in line.split("\t")]
            if len(fields) < 2:
                continue
            phrase = fields[0]
            syls = tuple(normalize_syllable(s) for s in fields[1].split())
            scope = fields[2] if len(fields) >= 3 else "substring"
            if len(syls) != len(phrase):
                continue
            if scope == "exact":
                self.poly_exact[phrase] = syls
            else:
                sub_map[phrase] = syls
        self.poly_sub = sorted(sub_map.items(), key=lambda item: (-len(item[0]), item[0]))

    def resolve(self, phrase: str) -> str:
        from lexicon_quality import corrected_reading

        if phrase in self.poly_exact:
            return corrected_reading(phrase, " ".join(self.poly_exact[phrase]))
        if phrase in self.exact_phrases:
            return corrected_reading(phrase, self.exact_phrases[phrase])

        out: list[str] = []
        i = 0
        n = len(phrase)
        while i < n:
            matched = False
            for sub, syls in self.poly_sub:
                if phrase.startswith(sub, i):
                    out.extend(syls)
                    i += len(sub)
                    matched = True
                    break
            if matched:
                continue
            for length in (3, 2):
                if i + length <= n:
                    chunk = phrase[i : i + length]
                    if chunk in self.sub_phrases:
                        out.extend(self.sub_phrases[chunk])
                        i += length
                        matched = True
                        break
            if matched:
                continue
            ch = phrase[i]
            py = self.char_primary.get(ch)
            if py is None:
                options = sorted(self.char_all.get(ch, set()))
                if not options:
                    raise ValueError(f"No pinyin reading for char {ch!r} in {phrase!r}")
                py = options[0]
            out.append(py)
            i += 1
        return corrected_reading(phrase, normalize_reading(" ".join(out)))


def parse_seed_file(path: Path) -> list[tuple[str, str | None, str]]:
    """Parse seed entries; explicit tab rows take precedence over pipe entries."""
    tier = "high"
    raw_items: list[tuple[str, str | None, str]] = []
    explicit_phrases: set[str] = set()
    for raw in path.read_text(encoding="utf-8-sig").splitlines():
        line = raw.strip()
        if not line or line.startswith("#"):
            continue
        if line.startswith("[") and line.endswith("]"):
            tier = line[1:-1].strip().lower()
            continue
        if "\t" in line:
            parts = [p.strip() for p in line.split("\t")]
            phrase = parts[0]
            code = parts[1] if len(parts) >= 2 and parts[1] else None
            row_tier = parts[2].lower() if len(parts) >= 3 and parts[2] else tier
            if phrase:
                raw_items.append((phrase, code, row_tier))
                if code is not None:
                    explicit_phrases.add(phrase)
        elif "|" in line:
            for token in line.split("|"):
                phrase = token.strip()
                if phrase:
                    raw_items.append((phrase, None, tier))
        else:
            raw_items.append((line, None, tier))

    # Drop implicit pipe occurrence when an explicit reading exists in the same file
    return [
        (phrase, code, row_tier)
        for phrase, code, row_tier in raw_items
        if code is not None or phrase not in explicit_phrases
    ]


def load_zh_best_by_py(root: Path) -> dict[str, tuple[int, str]]:
    zh_best: dict[str, tuple[int, str]] = {}
    for path in sorted((root / "lexicon" / "zh").glob("*.txt")):
        if path.name == "single_char_common_8105.txt":
            continue
        for line in path.read_text(encoding="utf-8-sig").splitlines():
            if not line.strip() or line.startswith("#"):
                continue
            fields = line.split("\t")
            if len(fields) < 3:
                continue
            phrase, py, score = fields[0], fields[1], int(fields[2])
            prev = zh_best.get(py)
            if prev is None or score > prev[0]:
                zh_best[py] = (score, phrase)
    return zh_best


def load_default_ext_max_by_py(root: Path) -> dict[str, tuple[int, str]]:
    manifest = json.loads((root / "shared" / "category_lexicons.json").read_text(encoding="utf-8"))
    ext_best: dict[str, tuple[int, str]] = {}
    for item in manifest["categories"]:
        if not item.get("default_enabled", False):
            continue
        path = root / "lexicon" / "zh-ext" / item["file"]
        if not path.is_file():
            continue
        for line in path.read_text(encoding="utf-8-sig").splitlines():
            if not line.strip() or line.startswith("#"):
                continue
            fields = line.split("\t")
            if len(fields) < 3:
                continue
            phrase, py, score = fields[0], fields[1], int(fields[2])
            prev = ext_best.get(py)
            if prev is None or score > prev[0]:
                ext_best[py] = (score, phrase)
    return ext_best


def read_lexicon_file(path: Path) -> tuple[list[str], list[tuple[str, str, int]]]:
    headers: list[str] = []
    rows: list[tuple[str, str, int]] = []
    for line in path.read_text(encoding="utf-8-sig").splitlines():
        if not line.strip():
            continue
        if line.startswith("#"):
            headers.append(line)
            continue
        fields = line.split("\t")
        rows.append((fields[0], fields[1], int(fields[2])))
    return headers, rows


def expand_ext_category(
    target_rel: str,
    seed_path: Path,
    resolver: OfflinePinyinResolver,
    zh_best: dict[str, tuple[int, str]],
    valid_syllables: set[str],
) -> tuple[int, int, int]:
    from lexicon_quality import (
        FULLWIDTH_PUNCT_RE,
        canonical_entry,
        character_readings,
        phrase_rejection_reason,
        reading_matches_characters,
    )

    reference = character_readings(ROOT)
    target_path = ROOT / target_rel
    headers, existing_rows = read_lexicon_file(target_path)
    before = len(existing_rows)

    merged: OrderedDict[tuple[str, str], int] = OrderedDict()
    existing_phrases: set[str] = set()
    for phrase, reading, score in existing_rows:
        merged[(phrase, reading)] = max(merged.get((phrase, reading), 0), score)
        existing_phrases.add(phrase)

    added = 0
    for phrase, explicit_code, tier in parse_seed_file(seed_path):
        # Category lists hold multi-char phrases/entities, not standalone single chars
        if len(phrase) < 2 or FULLWIDTH_PUNCT_RE.search(phrase):
            continue
        if explicit_code is not None:
            code = normalize_reading(explicit_code)
        else:
            if not HANZI_RE.match(phrase):
                raise ValueError(f"{seed_path}: non-Hanzi phrase needs explicit code: {phrase!r}")
            code = resolver.resolve(phrase)

        phrase, code = canonical_entry(phrase, code)
        if phrase_rejection_reason(phrase):
            continue
        if HANZI_RE.match(phrase):
            syls = code.split()
            if len(syls) != len(phrase) or any(s not in valid_syllables for s in syls):
                raise ValueError(f"{seed_path}: invalid syllables {phrase!r} -> {code!r}")
            if not reading_matches_characters(phrase, code, reference):
                raise ValueError(f"{seed_path}: reading mismatch {phrase!r} -> {code!r}")
        elif len(code.strip()) <= 1:
            raise ValueError(f"{seed_path}: single-char code for {phrase!r} -> {code!r}")

        # Avoid duplicating an existing phrase with an accidental alternate reading
        if (phrase, code) in merged or phrase in existing_phrases:
            continue

        score = EXT_TIER_SCORES.get(tier, 3600)
        best = zh_best.get(code)
        if best is not None and best[1] != phrase:
            score = min(score, max(100, best[0] + 2800))
        merged[(phrase, code)] = score
        existing_phrases.add(phrase)
        added += 1

    sorted_rows = sorted(merged.items(), key=lambda kv: (-kv[1], kv[0][0], kv[0][1]))
    if headers:
        headers[0] = re.sub(r"\d+(?=\s*(?:条|个))", str(len(sorted_rows)), headers[0])
    content = (
        "\n".join(headers)
        + "\n"
        + "".join(f"{phrase}\t{code}\t{score}\n" for (phrase, code), score in sorted_rows)
    )
    target_path.write_text(content, encoding="utf-8", newline="\n")
    return before, len(sorted_rows), added


def expand_four_char_base(target_count: int = 12_500) -> tuple[int, int, int]:
    """Expand lexicon/zh/life_common_4char.txt from 5,000 to target_count."""
    from lexicon_quality import (
        canonical_entry,
        character_readings,
        phrase_rejection_reason,
        reading_matches_characters,
    )

    reference = character_readings(ROOT)
    default_ext_best = load_default_ext_max_by_py(ROOT)

    common_chars: set[str] = set()
    single_path = ROOT / "lexicon" / "zh" / "single_char_common_8105.txt"
    for line in single_path.read_text(encoding="utf-8-sig").splitlines():
        if line.strip() and not line.startswith("#"):
            common_chars.add(line.split("\t")[0])
            if len(common_chars) >= 3800:
                break

    four_path = ROOT / "lexicon" / "zh" / "life_common_4char.txt"
    headers, existing_rows = read_lexicon_file(four_path)
    before = len(existing_rows)

    existing_keys = {(p, r) for p, r, _ in existing_rows}
    existing_phrases = {p for p, _, _ in existing_rows}

    noisy_substrings = (
        "九阴",
        "打狗棒",
        "葵花宝",
        "独孤九",
        "降龙十",
        "吸星大",
        "乾坤大",
        "北冥神",
        "凌波微",
        "六脉神",
        "如来神",
        "玉女心",
        "蛤蟆功",
        "辟邪剑",
        "太极剑",
        "少林寺",
        "武当派",
        "峨眉派",
        "华山派",
        "嵩山派",
        "日月神",
        "明教",
        "丐帮",
        "和钟",
        "帝制",
        "奴隶主",
        "封建主",
        "买办",
        "地主阶",
        "剥削阶",
        "红卫兵",
        "大字报",
        "走资派",
        "反革命",
        "肃反",
        "整风",
        "批斗",
        "军阀",
        "土匪",
        "汉奸",
        "伪军",
        "国军",
        "日军",
        "美军",
        "苏军",
        "法西斯",
        "纳粹",
        "盖世太",
        "克格勃",
        "中情局",
        "联调局",
        "操你",
        "傻逼",
        "他妈",
        "你妈",
        "妈逼",
        "狗日",
        "王八",
        "死鬼",
        "杂种",
    )

    def is_clean_modern_four(phrase: str, reading: str, proposed_score: int) -> bool:
        if len(phrase) != 4 or not HANZI_RE.match(phrase):
            return False
        if phrase in existing_phrases or (phrase, reading) in existing_keys:
            return False
        if phrase_rejection_reason(phrase):
            return False
        if any(bad in phrase for bad in noisy_substrings):
            return False
        if any(ch not in common_chars for ch in phrase):
            return False
        if not reading_matches_characters(phrase, reading, reference):
            return False
        if phrase[0] in "的了着过吗呢吧啊嘛呀哇啦哦嗯" or phrase[-1] in "的了着过吗呢吧啊嘛呀哇啦":
            return False
        # Prevent introducing a low-score zh homophone that causes an existing
        # default-enabled zh-ext entry to trip the >3500 ranking gap invariant.
        ext_top = default_ext_best.get(reading)
        if ext_top is not None and ext_top[1] != phrase and ext_top[0] - proposed_score > 3400:
            return False
        return True

    def collect_candidates(path: Path) -> list[tuple[str, str, int]]:
        out: list[tuple[str, str, int]] = []
        _, rows = read_lexicon_file(path)
        for p, r, s in rows:
            p, r = canonical_entry(p, r)
            out.append((p, r, s))
        return out

    idiom_pool = collect_candidates(ROOT / "lexicon" / "zh-ext" / "category_idioms.txt")
    common4_pool = collect_candidates(ROOT / "lexicon" / "zh-ext" / "category_common_four.txt")
    curated4_pool: list[tuple[str, str, int]] = []
    for ext_name in (
        "daily_communication.txt",
        "category_office.txt",
        "category_dining.txt",
        "category_brands.txt",
    ):
        curated4_pool.extend(collect_candidates(ROOT / "lexicon" / "zh-ext" / ext_name))

    need = max(0, target_count - before)
    chosen: OrderedDict[tuple[str, str], int] = OrderedDict()
    chosen_phrases: set[str] = set()

    # 1) Curated everyday 4-char phrases from default-on expansions (up to 1,500)
    for p, r, _ in curated4_pool:
        if len(chosen) >= min(need, 1500):
            break
        if p in chosen_phrases or not is_clean_modern_four(p, r, 3300):
            continue
        chosen[(p, r)] = 3300
        chosen_phrases.add(p)

    # 2) Top clean idioms (up to 3,600)
    idiom_target = min(need - len(chosen), 3600)
    idiom_added = 0
    for idx, (p, r, _) in enumerate(idiom_pool):
        if idiom_added >= idiom_target:
            break
        score = max(1200, 3150 - round(idx * 1950 / max(1, idiom_target)))
        if p in chosen_phrases or not is_clean_modern_four(p, r, score):
            continue
        chosen[(p, r)] = score
        chosen_phrases.add(p)
        idiom_added += 1

    # 3) Top clean modern 4-char general words (fill remaining to target_count)
    rem_target = max(0, need - len(chosen))
    rem_added = 0
    for idx, (p, r, _) in enumerate(common4_pool):
        if rem_added >= rem_target:
            break
        score = max(200, 2600 - round(idx * 2400 / max(1, rem_target)))
        if p in chosen_phrases or not is_clean_modern_four(p, r, score):
            continue
        chosen[(p, r)] = score
        chosen_phrases.add(p)
        rem_added += 1

    all_rows = list(existing_rows) + [(p, r, s) for (p, r), s in chosen.items()]
    all_rows.sort(key=lambda row: (-row[2], row[0], row[1]))

    if headers:
        headers[0] = f"# 开心输入法原生词库：{len(all_rows)} 条通用四字短语"
    content = "\n".join(headers) + "\n" + "".join(f"{p}\t{r}\t{s}\n" for p, r, s in all_rows)
    four_path.write_text(content, encoding="utf-8", newline="\n")
    return before, len(all_rows), len(chosen)


def expand_modern_and_seasonal(
    seed_path: Path,
    resolver: OfflinePinyinResolver,
    valid_syllables: set[str],
) -> tuple[tuple[int, int, int], tuple[int, int, int]]:
    from lexicon_quality import (
        canonical_entry,
        character_readings,
        phrase_rejection_reason,
        reading_matches_characters,
    )

    reference = character_readings(ROOT)

    base_keys: dict[tuple[str, str], str] = {}
    for rel in (
        "lexicon/zh/life_common_2char_125000.txt",
        "lexicon/zh/life_common_3char.txt",
        "lexicon/zh/life_common_4char.txt",
    ):
        _, rows = read_lexicon_file(ROOT / rel)
        for p, r, _ in rows:
            base_keys[(p, r)] = rel

    def load_tsv(path: Path) -> tuple[list[str], list[list[str]]]:
        headers: list[str] = []
        rows: list[list[str]] = []
        for line in path.read_text(encoding="utf-8-sig").splitlines():
            if not line.strip():
                continue
            if line.startswith("#"):
                headers.append(line)
                continue
            rows.append(line.split("\t"))
        return headers, rows

    modern_path = ROOT / "data_sources" / "kaixin" / "modern_life.tsv"
    seasonal_path = ROOT / "data_sources" / "kaixin" / "seasonal_hot_terms.tsv"
    m_headers, m_rows = load_tsv(modern_path)
    s_headers, s_rows = load_tsv(seasonal_path)
    m_before, s_before = len(m_rows), len(s_rows)

    m_seen = {(r[0], r[1]) for r in m_rows}
    s_seen = {(r[0], r[1]) for r in s_rows}

    m_added = 0
    s_added = 0
    for phrase, explicit_code, section in parse_seed_file(seed_path):
        if not HANZI_RE.match(phrase) or not 2 <= len(phrase) <= 10:
            continue
        code = normalize_reading(explicit_code) if explicit_code else resolver.resolve(phrase)
        phrase, code = canonical_entry(phrase, code)
        if phrase_rejection_reason(phrase):
            continue
        syls = code.split()
        if len(syls) != len(phrase) or any(s not in valid_syllables for s in syls):
            raise ValueError(f"{seed_path}: invalid syllables {phrase!r} -> {code!r}")
        if not reading_matches_characters(phrase, code, reference):
            raise ValueError(f"{seed_path}: reading mismatch {phrase!r} -> {code!r}")

        key = (phrase, code)
        if section == "modern":
            if key in m_seen or key in s_seen or key in base_keys:
                continue
            m_rows.append(
                [
                    phrase,
                    code,
                    "6200",
                    "-",
                    "2024-2026现代生活与科技热词；人工整理优先级，非实测词频",
                ]
            )
            m_seen.add(key)
            m_added += 1
        elif section == "seasonal":
            if key in s_seen or key in m_seen or key in base_keys:
                continue
            s_rows.append(
                [
                    phrase,
                    code,
                    "4600",
                    "-",
                    "2024-2026阶段网络热词；温和权重，按季度复审与清理",
                ]
            )
            s_seen.add(key)
            s_added += 1

    modern_path.write_text(
        "\n".join(m_headers) + "\n" + "".join("\t".join(r) + "\n" for r in m_rows),
        encoding="utf-8",
        newline="\n",
    )
    seasonal_path.write_text(
        "\n".join(s_headers) + "\n" + "".join("\t".join(r) + "\n" for r in s_rows),
        encoding="utf-8",
        newline="\n",
    )
    return (m_before, len(m_rows), m_added), (s_before, len(s_rows), s_added)


def main() -> int:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--four-char-target", type=int, default=12_500)
    args = parser.parse_args()

    valid_syllables = {
        line.strip().lower()
        for line in (ROOT / "pinyin-ime" / "data" / "syllables.txt")
        .read_text(encoding="utf-8-sig")
        .splitlines()
        if line.strip()
    }
    resolver = OfflinePinyinResolver(ROOT)
    zh_best = load_zh_best_by_py(ROOT)

    for rel, seed_name in (
        ("lexicon/zh-ext/category_brands.txt", "brands_seed.txt"),
        ("lexicon/zh-ext/category_dining.txt", "dining_seed.txt"),
        ("lexicon/zh-ext/daily_communication.txt", "daily_chat_seed.txt"),
        ("lexicon/zh-ext/category_office.txt", "office_seed.txt"),
    ):
        before, after, added = expand_ext_category(
            rel, PHASE1_DIR / seed_name, resolver, zh_best, valid_syllables
        )
        print(f"{rel}: {before} -> {after} (+{added})")

    f_before, f_after, f_added = expand_four_char_base(args.four_char_target)
    print(f"lexicon/zh/life_common_4char.txt: {f_before} -> {f_after} (+{f_added})")

    (m_b, m_a, m_add), (s_b, s_a, s_add) = expand_modern_and_seasonal(
        PHASE1_DIR / "modern_hot_seed.txt", resolver, valid_syllables
    )
    print(f"data_sources/kaixin/modern_life.tsv: {m_b} -> {m_a} (+{m_add})")
    print(f"data_sources/kaixin/seasonal_hot_terms.tsv: {s_b} -> {s_a} (+{s_add})")

    from lexicon_quality import apply_quality, check_quality

    report = apply_quality()
    print(
        f"lexicon_quality --apply: removed={len(report['removed'])}, "
        f"corrected={len(report['corrected'])}, "
        f"deduplicated={len(report['deduplicated'])}"
    )
    errors = check_quality(ROOT)
    if errors:
        for err in errors[:30]:
            print("ERROR:", err)
        print(f"Total quality errors: {len(errors)}")
        return 1
    print("All lexicon quality checks PASSED!")
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
