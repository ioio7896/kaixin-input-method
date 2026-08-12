//! `vv` 双前缀辅助：日期、时间、金额、剪贴板、符号、Markdown 片段等（完全离线）。
//! 同时暴露 `rq` / `sj` / `xq` 的直达候选，供普通拼音输入直接置顶显示。

use crate::clipboard_store;
use chrono::{DateTime, Datelike, FixedOffset, Utc};
use std::collections::HashSet;
use std::path::PathBuf;
use std::time::{SystemTime, UNIX_EPOCH};

const DIGIT_CN: [&str; 10] = ["零", "一", "二", "三", "四", "五", "六", "七", "八", "九"];
const DIGIT_UPPER: [&str; 10] = ["零", "壹", "贰", "叁", "肆", "伍", "陆", "柒", "捌", "玖"];
const BEIJING_UTC_OFFSET_SECS: i32 = 8 * 60 * 60;
const DIRECT_SHORTCUT_TOP_SCORE: f64 = 10_000.0;
const DIRECT_SHORTCUT_STEP: f64 = 50.0;
const CLIPBOARD_QUICK_LIMIT: usize = 8;
const CLIPBOARD_QUICK_PINNED_LIMIT: usize = 3;
const CLIPBOARD_QUICK_PREVIEW_CHARS: usize = 40;
const CLIPBOARD_QUICK_PREVIEW_LINES: usize = 3;
const CLIPBOARD_QUICK_PREVIEW_LINE_CHARS: usize = 56;
const CLIPBOARD_QUICK_INLINE_TEXT_UTF16_LIMIT: usize = 500;
const CLIPBOARD_DISPLAY_META_PREFIX: &str = "display=";
const DIRECT_NO_LEARN_META: &str = "no_learn=1";
const CLIPBOARD_NO_LEARN_META: &str = DIRECT_NO_LEARN_META;
const CLIPBOARD_QUICK_META: &str = "clipboard_quick=1";
const CLIPBOARD_ID_META_PREFIX: &str = "clipboard_key=";
const CLIPBOARD_VERTICAL_LAYOUT_META: &str = "layout=vertical";
const CLIPBOARD_PAGE_META_PREFIX: &str = "clipboard_page=";
const CLIPBOARD_PAGES_META_PREFIX: &str = "clipboard_pages=";
const CLIPBOARD_PINNED_META: &str = "clipboard_pinned=1";
const DATE_SHORTCUT_KEYS: &[&str] = &[
    "rq", "date", "jr", "sj", "time", "dt", "now", "xq", "week", "zhou",
];
const VV_COMMAND_ALIASES: &[(&str, &[&str])] = &[
    ("week", &["xq", "week", "zhou"]),
    ("symbol", &["sym", "fh", "symbol"]),
    ("emoji", &["emoji", "emjio", "face", "bq", "biaoqing"]),
    ("unit", &["unit", "dw"]),
    ("currency", &["dx", "daxie", "rmb", "money", "je"]),
    ("email", &["mail", "email"]),
    ("url", &["url", "site", "http", "https"]),
    ("markdown", &["md", "markdown"]),
    ("clipboard", &["cb", "clip", "clipboard", "paste"]),
    ("handwrite", &["hw", "handwrite", "sx", "shouxie"]),
];

#[derive(Clone, Copy)]
struct SymbolItem {
    symbol: &'static str,
    name: &'static str,
    group: &'static str,
    aliases: &'static [&'static str],
}

#[derive(Clone, Copy)]
struct EmojiItem {
    emoji: &'static str,
    name: &'static str,
    group: &'static str,
    aliases: &'static [&'static str],
}

const SYMBOL_GROUP_ALIASES: &[(&str, &[&str])] = &[
    ("punct", &["punctuation", "bd", "biaodian", "mark"]),
    ("quote", &["quotes", "yh", "yinhao"]),
    (
        "bracket",
        &["brackets", "paren", "parenthesis", "kh", "kuohao"],
    ),
    ("arrow", &["arrows", "jt", "jiantou"]),
    ("math", &["sx", "shuxue"]),
    ("shape", &["box", "shapes", "tx", "tuxing"]),
    ("num", &["number", "numbers", "xh", "xuhao", "bianhao"]),
];

const SYMBOL_ITEMS: &[SymbolItem] = &[
    SymbolItem {
        symbol: "，",
        name: "逗号",
        group: "punct",
        aliases: &["douhao", "dh", "comma", "cn_comma"],
    },
    SymbolItem {
        symbol: "、",
        name: "顿号",
        group: "punct",
        aliases: &["dunhao", "dhh", "pause"],
    },
    SymbolItem {
        symbol: "。",
        name: "句号",
        group: "punct",
        aliases: &["juhao", "jh", "period", "fullstop"],
    },
    SymbolItem {
        symbol: "；",
        name: "分号",
        group: "punct",
        aliases: &["fenhao", "fh", "semicolon"],
    },
    SymbolItem {
        symbol: "：",
        name: "冒号",
        group: "punct",
        aliases: &["maohao", "mh", "colon"],
    },
    SymbolItem {
        symbol: "？",
        name: "问号",
        group: "punct",
        aliases: &["wenhao", "wh", "question"],
    },
    SymbolItem {
        symbol: "！",
        name: "感叹号",
        group: "punct",
        aliases: &["gantanhao", "gth", "exclamation"],
    },
    SymbolItem {
        symbol: "……",
        name: "省略号",
        group: "punct",
        aliases: &["shengluehao", "slh", "ellipsis"],
    },
    SymbolItem {
        symbol: "—",
        name: "破折号",
        group: "punct",
        aliases: &["pozhehao", "pzh", "dash", "emdash"],
    },
    SymbolItem {
        symbol: "·",
        name: "间隔号",
        group: "punct",
        aliases: &["jiangehao", "jgh", "middle_dot"],
    },
    SymbolItem {
        symbol: "～",
        name: "波浪号",
        group: "punct",
        aliases: &["bolanghao", "blh", "tilde"],
    },
    SymbolItem {
        symbol: ",",
        name: "英文逗号",
        group: "punct",
        aliases: &["comma_en", "en_comma", "ascii_comma"],
    },
    SymbolItem {
        symbol: ".",
        name: "英文句号",
        group: "punct",
        aliases: &["period_en", "dot", "ascii_period"],
    },
    SymbolItem {
        symbol: "“",
        name: "左双引号",
        group: "quote",
        aliases: &["zuoshuangyinhao", "left_quote", "lquote"],
    },
    SymbolItem {
        symbol: "”",
        name: "右双引号",
        group: "quote",
        aliases: &["youshuangyinhao", "right_quote", "rquote"],
    },
    SymbolItem {
        symbol: "‘",
        name: "左单引号",
        group: "quote",
        aliases: &["zuodanyinhao", "left_single_quote"],
    },
    SymbolItem {
        symbol: "’",
        name: "右单引号",
        group: "quote",
        aliases: &["youdanyinhao", "right_single_quote"],
    },
    SymbolItem {
        symbol: "\"",
        name: "英文双引号",
        group: "quote",
        aliases: &["quote", "double_quote"],
    },
    SymbolItem {
        symbol: "'",
        name: "英文单引号",
        group: "quote",
        aliases: &["apostrophe", "single_quote"],
    },
    SymbolItem {
        symbol: "（",
        name: "左圆括号",
        group: "bracket",
        aliases: &["zuoyuankuohao", "left_paren"],
    },
    SymbolItem {
        symbol: "）",
        name: "右圆括号",
        group: "bracket",
        aliases: &["youyuankuohao", "right_paren"],
    },
    SymbolItem {
        symbol: "【",
        name: "左方头括号",
        group: "bracket",
        aliases: &["zuofangtoukuohao", "left_black_bracket"],
    },
    SymbolItem {
        symbol: "】",
        name: "右方头括号",
        group: "bracket",
        aliases: &["youfangtoukuohao", "right_black_bracket"],
    },
    SymbolItem {
        symbol: "《",
        name: "左书名号",
        group: "bracket",
        aliases: &["zuoshuminghao", "left_book_title"],
    },
    SymbolItem {
        symbol: "》",
        name: "右书名号",
        group: "bracket",
        aliases: &["youshuminghao", "right_book_title"],
    },
    SymbolItem {
        symbol: "「",
        name: "左直角引号",
        group: "bracket",
        aliases: &["zuozhijiao", "left_corner"],
    },
    SymbolItem {
        symbol: "」",
        name: "右直角引号",
        group: "bracket",
        aliases: &["youzhijiao", "right_corner"],
    },
    SymbolItem {
        symbol: "〔",
        name: "左六角括号",
        group: "bracket",
        aliases: &["zuoliujiao", "left_tortoise"],
    },
    SymbolItem {
        symbol: "〕",
        name: "右六角括号",
        group: "bracket",
        aliases: &["youliujiao", "right_tortoise"],
    },
    SymbolItem {
        symbol: "←",
        name: "左箭头",
        group: "arrow",
        aliases: &["zuojiantou", "left_arrow"],
    },
    SymbolItem {
        symbol: "↑",
        name: "上箭头",
        group: "arrow",
        aliases: &["shangjiantou", "up_arrow"],
    },
    SymbolItem {
        symbol: "→",
        name: "右箭头",
        group: "arrow",
        aliases: &["youjiantou", "right_arrow"],
    },
    SymbolItem {
        symbol: "↓",
        name: "下箭头",
        group: "arrow",
        aliases: &["xiajiantou", "down_arrow"],
    },
    SymbolItem {
        symbol: "±",
        name: "正负号",
        group: "math",
        aliases: &["zhengfuhao", "plus_minus"],
    },
    SymbolItem {
        symbol: "×",
        name: "乘号",
        group: "math",
        aliases: &["chenghao", "multiply", "times"],
    },
    SymbolItem {
        symbol: "÷",
        name: "除号",
        group: "math",
        aliases: &["chuhao", "divide"],
    },
    SymbolItem {
        symbol: "≈",
        name: "约等于",
        group: "math",
        aliases: &["yuedengyu", "approx"],
    },
    SymbolItem {
        symbol: "≠",
        name: "不等于",
        group: "math",
        aliases: &["budengyu", "not_equal"],
    },
    SymbolItem {
        symbol: "≤",
        name: "小于等于",
        group: "math",
        aliases: &["xiaoyudengyu", "less_equal"],
    },
    SymbolItem {
        symbol: "≥",
        name: "大于等于",
        group: "math",
        aliases: &["dayudengyu", "greater_equal"],
    },
    SymbolItem {
        symbol: "∞",
        name: "无穷大",
        group: "math",
        aliases: &["wuqiongda", "infinity"],
    },
    SymbolItem {
        symbol: "✓",
        name: "对勾",
        group: "shape",
        aliases: &["duigou", "check"],
    },
    SymbolItem {
        symbol: "✔",
        name: "粗对勾",
        group: "shape",
        aliases: &["cuduigou", "heavy_check"],
    },
    SymbolItem {
        symbol: "☆",
        name: "空心星",
        group: "shape",
        aliases: &["kongxinxing", "star"],
    },
    SymbolItem {
        symbol: "★",
        name: "实心星",
        group: "shape",
        aliases: &["shixinxing", "black_star"],
    },
    SymbolItem {
        symbol: "①",
        name: "圈一",
        group: "num",
        aliases: &["quanyi", "circled_1"],
    },
    SymbolItem {
        symbol: "②",
        name: "圈二",
        group: "num",
        aliases: &["quaner", "circled_2"],
    },
    SymbolItem {
        symbol: "③",
        name: "圈三",
        group: "num",
        aliases: &["quansan", "circled_3"],
    },
    SymbolItem {
        symbol: "⑴",
        name: "括号一",
        group: "num",
        aliases: &["kuohaoyi", "paren_1"],
    },
    SymbolItem {
        symbol: "Ⅰ",
        name: "罗马一",
        group: "num",
        aliases: &["luomayi", "roman_1"],
    },
];

const EMOJI_GROUP_ALIASES: &[(&str, &[&str])] = &[
    (
        "face",
        &["faces", "smile", "smiley", "bq", "biaoqing", "表情"],
    ),
    ("hand", &["hands", "gesture", "ss", "shoushi", "手势"]),
    (
        "heart",
        &["love", "xin", "aixin", "heart", "hearts", "爱心"],
    ),
    ("work", &["office", "status", "gz", "gongzuo", "办公"]),
    ("food", &["foods", "eat", "chi", "chide", "食物"]),
    ("weather", &["sky", "tianqi", "weather", "天气"]),
    ("animal", &["animals", "dw", "dongwu", "动物"]),
];

const DEFAULT_EMOJI_GROUPS: &[&str] = &["face", "hand", "heart", "work"];

const EMOJI_ITEMS: &[EmojiItem] = &[
    EmojiItem {
        emoji: "😀",
        name: "笑脸",
        group: "face",
        aliases: &["xiaolian", "smile", "grin"],
    },
    EmojiItem {
        emoji: "😄",
        name: "开心",
        group: "face",
        aliases: &["kaixin", "happy", "laugh"],
    },
    EmojiItem {
        emoji: "😂",
        name: "笑哭",
        group: "face",
        aliases: &["xiaoku", "lol", "joy"],
    },
    EmojiItem {
        emoji: "😊",
        name: "微笑",
        group: "face",
        aliases: &["weixiao", "smile_soft"],
    },
    EmojiItem {
        emoji: "😎",
        name: "酷",
        group: "face",
        aliases: &["ku", "cool"],
    },
    EmojiItem {
        emoji: "🙂",
        name: "浅笑",
        group: "face",
        aliases: &["qianxiao", "slight_smile"],
    },
    EmojiItem {
        emoji: "😉",
        name: "眨眼",
        group: "face",
        aliases: &["zhayan", "wink"],
    },
    EmojiItem {
        emoji: "😅",
        name: "汗笑",
        group: "face",
        aliases: &["hanxiao", "sweat_smile"],
    },
    EmojiItem {
        emoji: "😭",
        name: "大哭",
        group: "face",
        aliases: &["daku", "cry"],
    },
    EmojiItem {
        emoji: "🤔",
        name: "思考",
        group: "face",
        aliases: &["sikao", "think"],
    },
    EmojiItem {
        emoji: "😍",
        name: "喜欢",
        group: "face",
        aliases: &["xihuan", "love_face"],
    },
    EmojiItem {
        emoji: "😴",
        name: "睡觉",
        group: "face",
        aliases: &["shuijiao", "sleep"],
    },
    EmojiItem {
        emoji: "👍",
        name: "点赞",
        group: "hand",
        aliases: &["dianzan", "zan", "thumbs_up"],
    },
    EmojiItem {
        emoji: "👎",
        name: "点踩",
        group: "hand",
        aliases: &["diancai", "thumbs_down"],
    },
    EmojiItem {
        emoji: "👏",
        name: "鼓掌",
        group: "hand",
        aliases: &["guzhang", "clap"],
    },
    EmojiItem {
        emoji: "🙏",
        name: "感谢",
        group: "hand",
        aliases: &["ganxie", "thanks", "pray"],
    },
    EmojiItem {
        emoji: "👌",
        name: "好的",
        group: "hand",
        aliases: &["haode", "ok"],
    },
    EmojiItem {
        emoji: "👋",
        name: "挥手",
        group: "hand",
        aliases: &["huishou", "wave"],
    },
    EmojiItem {
        emoji: "🤝",
        name: "握手",
        group: "hand",
        aliases: &["woshou", "handshake"],
    },
    EmojiItem {
        emoji: "💪",
        name: "加油",
        group: "hand",
        aliases: &["jiayou", "strong"],
    },
    EmojiItem {
        emoji: "✌️",
        name: "胜利",
        group: "hand",
        aliases: &["shengli", "v", "victory"],
    },
    EmojiItem {
        emoji: "❤️",
        name: "红心",
        group: "heart",
        aliases: &["hongxin", "love", "heart"],
    },
    EmojiItem {
        emoji: "🧡",
        name: "橙心",
        group: "heart",
        aliases: &["chengxin", "orange_heart"],
    },
    EmojiItem {
        emoji: "💛",
        name: "黄心",
        group: "heart",
        aliases: &["huangxin", "yellow_heart"],
    },
    EmojiItem {
        emoji: "💚",
        name: "绿心",
        group: "heart",
        aliases: &["lvxin", "green_heart"],
    },
    EmojiItem {
        emoji: "💙",
        name: "蓝心",
        group: "heart",
        aliases: &["lanxin", "blue_heart"],
    },
    EmojiItem {
        emoji: "💜",
        name: "紫心",
        group: "heart",
        aliases: &["zixin", "purple_heart"],
    },
    EmojiItem {
        emoji: "💔",
        name: "心碎",
        group: "heart",
        aliases: &["xinsui", "broken_heart"],
    },
    EmojiItem {
        emoji: "✨",
        name: "闪光",
        group: "heart",
        aliases: &["shanguang", "sparkles"],
    },
    EmojiItem {
        emoji: "🔥",
        name: "火",
        group: "heart",
        aliases: &["huo", "fire"],
    },
    EmojiItem {
        emoji: "✅",
        name: "完成",
        group: "work",
        aliases: &["wancheng", "done", "check"],
    },
    EmojiItem {
        emoji: "❌",
        name: "错误",
        group: "work",
        aliases: &["cuowu", "wrong", "cross"],
    },
    EmojiItem {
        emoji: "⚠️",
        name: "警告",
        group: "work",
        aliases: &["jinggao", "warning"],
    },
    EmojiItem {
        emoji: "ℹ️",
        name: "信息",
        group: "work",
        aliases: &["xinxi", "info"],
    },
    EmojiItem {
        emoji: "💡",
        name: "灵感",
        group: "work",
        aliases: &["linggan", "idea"],
    },
    EmojiItem {
        emoji: "📌",
        name: "图钉",
        group: "work",
        aliases: &["tuding", "pin"],
    },
    EmojiItem {
        emoji: "📝",
        name: "记录",
        group: "work",
        aliases: &["jilu", "note"],
    },
    EmojiItem {
        emoji: "📅",
        name: "日历",
        group: "work",
        aliases: &["rili", "calendar"],
    },
    EmojiItem {
        emoji: "⏰",
        name: "闹钟",
        group: "work",
        aliases: &["naozhong", "alarm"],
    },
    EmojiItem {
        emoji: "🍎",
        name: "苹果",
        group: "food",
        aliases: &["pingguo", "apple"],
    },
    EmojiItem {
        emoji: "🍔",
        name: "汉堡",
        group: "food",
        aliases: &["hanbao", "burger"],
    },
    EmojiItem {
        emoji: "🍕",
        name: "披萨",
        group: "food",
        aliases: &["pisa", "pizza"],
    },
    EmojiItem {
        emoji: "🍜",
        name: "面条",
        group: "food",
        aliases: &["miantiao", "noodle"],
    },
    EmojiItem {
        emoji: "☕",
        name: "咖啡",
        group: "food",
        aliases: &["kafei", "coffee"],
    },
    EmojiItem {
        emoji: "☀️",
        name: "晴天",
        group: "weather",
        aliases: &["qingtian", "sun"],
    },
    EmojiItem {
        emoji: "🌧️",
        name: "下雨",
        group: "weather",
        aliases: &["xiayu", "rain"],
    },
    EmojiItem {
        emoji: "❄️",
        name: "下雪",
        group: "weather",
        aliases: &["xiaxue", "snow"],
    },
    EmojiItem {
        emoji: "🌈",
        name: "彩虹",
        group: "weather",
        aliases: &["caihong", "rainbow"],
    },
    EmojiItem {
        emoji: "☔",
        name: "雨伞",
        group: "weather",
        aliases: &["yusan", "umbrella"],
    },
    EmojiItem {
        emoji: "🐶",
        name: "小狗",
        group: "animal",
        aliases: &["xiaogou", "dog"],
    },
    EmojiItem {
        emoji: "🐱",
        name: "小猫",
        group: "animal",
        aliases: &["xiaomao", "cat"],
    },
    EmojiItem {
        emoji: "🐼",
        name: "熊猫",
        group: "animal",
        aliases: &["xiongmao", "panda"],
    },
    EmojiItem {
        emoji: "🐟",
        name: "鱼",
        group: "animal",
        aliases: &["yu", "fish"],
    },
    EmojiItem {
        emoji: "🦋",
        name: "蝴蝶",
        group: "animal",
        aliases: &["hudie", "butterfly"],
    },
];

#[derive(Clone, Debug, PartialEq)]
pub struct DirectCandidate {
    pub phrase: String,
    pub score: f64,
    pub meta: Option<String>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct LookupOptions {
    pub symbol_toolbox_enabled: bool,
    pub emoji_input_enabled: bool,
}

impl Default for LookupOptions {
    fn default() -> Self {
        Self {
            symbol_toolbox_enabled: true,
            emoji_input_enabled: true,
        }
    }
}

fn normalized_direct_input_key(input: &str) -> String {
    input.trim().to_ascii_lowercase()
}

fn is_datetime_shortcut_key(key: &str) -> bool {
    DATE_SHORTCUT_KEYS.contains(&key)
}

fn command_matches(cmd: &str, canonical: &str) -> bool {
    VV_COMMAND_ALIASES
        .iter()
        .find(|(name, _)| *name == canonical)
        .map(|(_, aliases)| aliases.contains(&cmd))
        .unwrap_or(false)
}

pub fn is_live_datetime_top_shortcut(input: &str) -> bool {
    matches!(normalized_direct_input_key(input).as_str(), "rq" | "sj")
}

fn beijing_now() -> DateTime<FixedOffset> {
    let offset = FixedOffset::east_opt(BEIJING_UTC_OFFSET_SECS).expect("valid Beijing UTC offset");
    Utc::now().with_timezone(&offset)
}

fn weekday_cn(now: &DateTime<FixedOffset>) -> &'static str {
    match now.weekday() {
        chrono::Weekday::Mon => "星期一",
        chrono::Weekday::Tue => "星期二",
        chrono::Weekday::Wed => "星期三",
        chrono::Weekday::Thu => "星期四",
        chrono::Weekday::Fri => "星期五",
        chrono::Weekday::Sat => "星期六",
        chrono::Weekday::Sun => "星期日",
    }
}

fn weekday_short_cn(now: &DateTime<FixedOffset>) -> &'static str {
    match now.weekday() {
        chrono::Weekday::Mon => "周一",
        chrono::Weekday::Tue => "周二",
        chrono::Weekday::Wed => "周三",
        chrono::Weekday::Thu => "周四",
        chrono::Weekday::Fri => "周五",
        chrono::Weekday::Sat => "周六",
        chrono::Weekday::Sun => "周日",
    }
}

fn weekday_libaicn(now: &DateTime<FixedOffset>) -> &'static str {
    match now.weekday() {
        chrono::Weekday::Mon => "礼拜一",
        chrono::Weekday::Tue => "礼拜二",
        chrono::Weekday::Wed => "礼拜三",
        chrono::Weekday::Thu => "礼拜四",
        chrono::Weekday::Fri => "礼拜五",
        chrono::Weekday::Sat => "礼拜六",
        chrono::Weekday::Sun => "礼拜天",
    }
}

fn split_command_args(input: &str) -> (&str, &str) {
    let trimmed = input.trim();
    if let Some(idx) = trimmed.find(char::is_whitespace) {
        (&trimmed[..idx], trimmed[idx..].trim())
    } else {
        (trimmed, "")
    }
}

fn score_candidates<I>(items: I, start_score: f64) -> Vec<(String, f64)>
where
    I: IntoIterator<Item = String>,
{
    let mut out = Vec::new();
    let mut seen = HashSet::new();
    for item in items {
        let trimmed = item.trim();
        if trimmed.is_empty() {
            continue;
        }
        let text = trimmed.to_string();
        if !seen.insert(text.clone()) {
            continue;
        }
        out.push((text, start_score - out.len() as f64));
    }
    out
}

fn direct_candidate(phrase: impl Into<String>, score: f64, meta: Option<&str>) -> DirectCandidate {
    DirectCandidate {
        phrase: phrase.into(),
        score,
        meta: meta.map(str::to_string),
    }
}

fn scored_to_direct(items: Vec<(String, f64)>) -> Vec<DirectCandidate> {
    items
        .into_iter()
        .map(|(phrase, score)| DirectCandidate {
            phrase,
            score,
            meta: None,
        })
        .collect()
}

fn datetime_candidates_for(key: &str) -> Option<Vec<DirectCandidate>> {
    let now = beijing_now();
    let full = now.format("%Y-%m-%d %H:%M:%S").to_string();
    let date = now.format("%Y-%m-%d").to_string();
    let time = now.format("%H:%M:%S").to_string();
    let week = weekday_cn(&now).to_string();
    let week_short = weekday_short_cn(&now).to_string();
    let week_libaicn = weekday_libaicn(&now).to_string();

    let out = match key {
        "rq" | "date" | "jr" => vec![
            direct_candidate(date.clone(), DIRECT_SHORTCUT_TOP_SCORE, Some("实时日期")),
            direct_candidate(
                full,
                DIRECT_SHORTCUT_TOP_SCORE - DIRECT_SHORTCUT_STEP,
                Some("北京时间"),
            ),
            direct_candidate(
                format!("{} {}", date, week),
                DIRECT_SHORTCUT_TOP_SCORE - DIRECT_SHORTCUT_STEP * 2.0,
                Some("实时日期"),
            ),
        ],
        "sj" | "time" => vec![
            direct_candidate(time.clone(), DIRECT_SHORTCUT_TOP_SCORE, Some("北京时间")),
            direct_candidate(
                full,
                DIRECT_SHORTCUT_TOP_SCORE - DIRECT_SHORTCUT_STEP,
                Some("北京时间"),
            ),
            direct_candidate(
                format!("北京时间 {}", time),
                DIRECT_SHORTCUT_TOP_SCORE - DIRECT_SHORTCUT_STEP * 2.0,
                Some("带时区说明"),
            ),
        ],
        "dt" | "now" => vec![
            direct_candidate(full, DIRECT_SHORTCUT_TOP_SCORE, Some("北京时间")),
            direct_candidate(
                date.clone(),
                DIRECT_SHORTCUT_TOP_SCORE - DIRECT_SHORTCUT_STEP,
                Some("实时日期"),
            ),
            direct_candidate(
                time,
                DIRECT_SHORTCUT_TOP_SCORE - DIRECT_SHORTCUT_STEP * 2.0,
                Some("北京时间"),
            ),
            direct_candidate(
                format!("{} {}", date, week_short),
                DIRECT_SHORTCUT_TOP_SCORE - DIRECT_SHORTCUT_STEP * 3.0,
                Some("实时日期"),
            ),
        ],
        "xq" | "week" | "zhou" => vec![
            direct_candidate(
                format!("{} {}", date, week),
                DIRECT_SHORTCUT_TOP_SCORE,
                Some("实时日期"),
            ),
            direct_candidate(
                format!("{} {}", date, week_short),
                DIRECT_SHORTCUT_TOP_SCORE - DIRECT_SHORTCUT_STEP,
                Some("实时日期"),
            ),
            direct_candidate(
                format!("{} {}", date, week_libaicn),
                DIRECT_SHORTCUT_TOP_SCORE - DIRECT_SHORTCUT_STEP * 2.0,
                Some("实时日期"),
            ),
        ],
        _ => return None,
    };
    Some(out)
}

fn function_key_candidates(key: &str) -> Option<Vec<DirectCandidate>> {
    if let Some(label) = named_function_key_label(key) {
        return Some(vec![direct_candidate(
            label,
            DIRECT_SHORTCUT_TOP_SCORE,
            Some("功能键"),
        )]);
    }

    let digits = key.strip_prefix('f')?;
    let number: u32 = digits.parse().ok()?;
    if !(1..=24).contains(&number) {
        return None;
    }
    Some(vec![direct_candidate(
        format!("F{number}"),
        DIRECT_SHORTCUT_TOP_SCORE,
        Some("功能键"),
    )])
}

fn named_function_key_label(key: &str) -> Option<&'static str> {
    match key {
        "enter" | "return" | "huiche" => Some("Enter"),
        "backspace" | "bksp" | "tuige" => Some("Backspace"),
        "delete" | "del" => Some("Delete"),
        "escape" | "esc" => Some("Esc"),
        "tab" => Some("Tab"),
        _ => None,
    }
}

pub fn is_function_key_shortcut(input: &str) -> bool {
    let key = input.trim().to_ascii_lowercase();
    function_key_candidates(key.as_str()).is_some()
}

pub fn is_direct_input_shortcut(input: &str) -> bool {
    let key = normalized_direct_input_key(input);
    function_key_candidates(key.as_str()).is_some() || is_datetime_shortcut_key(key.as_str())
}

pub fn direct_input_candidates_detailed(input: &str) -> Option<Vec<DirectCandidate>> {
    let key = normalized_direct_input_key(input);
    function_key_candidates(key.as_str()).or_else(|| datetime_candidates_for(key.as_str()))
}

pub fn direct_input_candidates(input: &str) -> Option<Vec<(String, f64)>> {
    direct_input_candidates_detailed(input).map(|items| {
        items
            .into_iter()
            .map(|item| (item.phrase, item.score))
            .collect()
    })
}

fn int_to_chinese(mut n: u64) -> String {
    if n == 0 {
        return DIGIT_CN[0].to_string();
    }
    let mut parts = Vec::new();
    let units = ["", "十", "百", "千"];
    let big = ["", "万", "亿"];
    let mut group = 0usize;
    while n > 0 {
        let mut g = (n % 10_000) as usize;
        n /= 10_000;
        if g == 0 {
            group += 1;
            continue;
        }
        let mut seg = String::new();
        let mut u = 0usize;
        let mut written_zero = false;
        while g > 0 {
            let d = g % 10;
            g /= 10;
            if d > 0 {
                seg.insert_str(0, units[u]);
                seg.insert_str(0, DIGIT_CN[d]);
                written_zero = false;
            } else if !seg.is_empty() && !written_zero {
                seg.insert_str(0, DIGIT_CN[0]);
                written_zero = true;
            }
            u += 1;
        }
        if group < big.len() && !big[group].is_empty() {
            seg.push_str(big[group]);
        }
        parts.insert(0, seg);
        group += 1;
    }
    let mut out = parts.join("");
    if out.starts_with("一十") && out.chars().count() > 1 {
        out = out.replacen("一十", "十", 1);
    }
    out
}

fn decimal_part_to_chinese(s: &str) -> String {
    s.chars()
        .filter_map(|c| c.to_digit(10).map(|d| DIGIT_CN[d as usize].to_string()))
        .collect()
}

fn sanitize_numeric_input(input: &str) -> Option<String> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut out = String::new();
    let mut seen_dot = false;
    for ch in trimmed.chars() {
        match ch {
            '0'..='9' => out.push(ch),
            '.' if !seen_dot => {
                out.push('.');
                seen_dot = true;
            }
            ' ' | ',' | '_' | '￥' | '¥' => {}
            _ => return None,
        }
    }
    if out.is_empty() || out == "." {
        return None;
    }
    Some(out)
}

fn parse_decimal_amount(input: &str) -> Option<(u64, u8, u8, String)> {
    let normalized = sanitize_numeric_input(input)?;
    let (int_part, frac_part) = normalized
        .split_once('.')
        .unwrap_or((normalized.as_str(), ""));
    if int_part.is_empty() || !int_part.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    if !frac_part.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let int_value = int_part.parse::<u64>().ok()?;
    let jiao = frac_part
        .chars()
        .next()
        .and_then(|c| c.to_digit(10))
        .unwrap_or(0) as u8;
    let fen = frac_part
        .chars()
        .nth(1)
        .and_then(|c| c.to_digit(10))
        .unwrap_or(0) as u8;
    let canonical = if frac_part.is_empty() {
        int_part.to_string()
    } else {
        format!("{int_part}.{}", &frac_part[..frac_part.len().min(2)])
    };
    Some((int_value, jiao, fen, canonical))
}

fn uppercase_group(group: u16) -> String {
    let digits = [
        (group / 1000) % 10,
        (group / 100) % 10,
        (group / 10) % 10,
        group % 10,
    ];
    let units = ["仟", "佰", "拾", ""];
    let mut out = String::new();
    let mut pending_zero = false;
    for (idx, digit) in digits.into_iter().enumerate() {
        if digit == 0 {
            if !out.is_empty() {
                pending_zero = true;
            }
            continue;
        }
        if pending_zero {
            out.push('零');
            pending_zero = false;
        }
        out.push_str(DIGIT_UPPER[digit as usize]);
        out.push_str(units[idx]);
    }
    out
}

fn uppercase_integer_to_rmb(mut value: u64) -> String {
    if value == 0 {
        return "零".to_string();
    }
    let big_units = ["", "万", "亿", "兆"];
    let mut groups = Vec::new();
    while value > 0 {
        groups.push((value % 10_000) as u16);
        value /= 10_000;
    }

    let mut out = String::new();
    let mut need_zero = false;
    for idx in (0..groups.len()).rev() {
        let group = groups[idx];
        if group == 0 {
            need_zero = !out.is_empty();
            continue;
        }
        if need_zero || (!out.is_empty() && group < 1000) {
            out.push('零');
        }
        out.push_str(&uppercase_group(group));
        out.push_str(big_units[idx]);
        need_zero = false;
    }
    out
}

fn uppercase_currency(amount: &str) -> Option<(String, String)> {
    let (int_value, jiao, fen, canonical) = parse_decimal_amount(amount)?;
    let mut out = uppercase_integer_to_rmb(int_value);
    out.push('元');
    match (jiao, fen) {
        (0, 0) => out.push('整'),
        (0, fen) => {
            out.push('零');
            out.push_str(DIGIT_UPPER[fen as usize]);
            out.push('分');
        }
        (jiao, 0) => {
            out.push_str(DIGIT_UPPER[jiao as usize]);
            out.push('角');
        }
        (jiao, fen) => {
            out.push_str(DIGIT_UPPER[jiao as usize]);
            out.push('角');
            out.push_str(DIGIT_UPPER[fen as usize]);
            out.push('分');
        }
    }
    Some((out, canonical))
}

fn numeric_candidates(input: &str) -> Option<Vec<(String, f64)>> {
    let normalized = sanitize_numeric_input(input)?;
    if !normalized.contains('.') && normalized.chars().all(|c| c.is_ascii_digit()) {
        let n = normalized.parse::<u64>().ok()?;
        let zh = int_to_chinese(n);
        let mut texts = vec![zh, format!("￥{normalized}")];
        if let Some((upper, _)) = uppercase_currency(&normalized) {
            texts.push(upper);
        }
        texts.push(normalized);
        return Some(score_candidates(texts, 100.0));
    }

    let (int_value, _, _, canonical) = parse_decimal_amount(&normalized)?;
    let zh_int = int_to_chinese(int_value);
    let frac = canonical
        .split_once('.')
        .map(|(_, rhs)| rhs)
        .unwrap_or_default();
    let zh = if frac.is_empty() {
        format!("{zh_int}元整")
    } else {
        format!("{zh_int}点{}元", decimal_part_to_chinese(frac))
    };
    let mut texts = vec![zh, format!("￥{canonical}")];
    if let Some((upper, _)) = uppercase_currency(&canonical) {
        texts.push(upper);
    }
    texts.push(canonical);
    Some(score_candidates(texts, 100.0))
}

fn weekday_candidates() -> Vec<(String, f64)> {
    let now = beijing_now();
    let date = now.format("%Y-%m-%d").to_string();
    score_candidates(
        vec![
            format!("{date} {}", weekday_cn(&now)),
            format!("{date} {}", weekday_short_cn(&now)),
            format!("{date} {}", weekday_libaicn(&now)),
        ],
        120.0,
    )
}

fn symbol_group_matches(group: &str, key: &str) -> bool {
    group == key
        || SYMBOL_GROUP_ALIASES
            .iter()
            .find(|(name, _)| *name == group)
            .is_some_and(|(_, aliases)| aliases.contains(&key))
}

fn symbol_alias_matches(alias: &str, key: &str) -> bool {
    alias == key || (key.is_ascii() && key.len() >= 2 && alias.starts_with(key))
}

fn symbol_item_matches(item: &SymbolItem, key: &str) -> bool {
    if key.is_empty() {
        return true;
    }
    item.symbol == key
        || item.name.contains(key)
        || symbol_group_matches(item.group, key)
        || item
            .aliases
            .iter()
            .any(|alias| symbol_alias_matches(alias, key))
}

fn symbol_meta(item: &SymbolItem) -> String {
    format!("符号: {}", item.name)
}

fn emoji_group_matches(group: &str, key: &str) -> bool {
    group.eq_ignore_ascii_case(key)
        || EMOJI_GROUP_ALIASES
            .iter()
            .any(|(canonical, aliases)| *canonical == group && aliases.contains(&key))
}

fn emoji_item_match_rank(item: &EmojiItem, key: &str) -> Option<u8> {
    if key.is_empty() {
        return None;
    }
    if item.emoji == key || item.name == key || item.aliases.contains(&key) {
        return Some(0);
    }
    if item.name.contains(key) || item.aliases.iter().any(|alias| alias.contains(key)) {
        return Some(1);
    }
    if emoji_group_matches(item.group, key) {
        return Some(2);
    }
    None
}

fn emoji_meta(item: &EmojiItem) -> String {
    format!("Emoji: {}\t{}", item.name, DIRECT_NO_LEARN_META)
}

fn emoji_panel_candidates_detailed(arg: &str) -> Vec<DirectCandidate> {
    let key = arg.trim().to_ascii_lowercase();
    let mut matched = if key.is_empty() {
        EMOJI_ITEMS
            .iter()
            .enumerate()
            .filter(|(_, item)| DEFAULT_EMOJI_GROUPS.contains(&item.group))
            .map(|(index, item)| (2, index, item))
            .collect::<Vec<_>>()
    } else {
        EMOJI_ITEMS
            .iter()
            .enumerate()
            .filter_map(|(index, item)| {
                emoji_item_match_rank(item, &key).map(|rank| (rank, index, item))
            })
            .collect::<Vec<_>>()
    };

    if matched.is_empty() {
        matched = EMOJI_ITEMS
            .iter()
            .enumerate()
            .filter(|(_, item)| DEFAULT_EMOJI_GROUPS.contains(&item.group))
            .map(|(index, item)| (3, index, item))
            .collect();
    }
    matched.sort_by_key(|(rank, index, _)| (*rank, *index));

    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for (_, _, item) in matched {
        if !seen.insert(item.emoji) {
            continue;
        }
        out.push(DirectCandidate {
            phrase: item.emoji.to_string(),
            score: 148.0 - out.len() as f64,
            meta: Some(emoji_meta(item)),
        });
    }
    out
}

fn symbol_panel_candidates_detailed(arg: &str) -> Vec<DirectCandidate> {
    let key = arg.trim().to_ascii_lowercase();
    if matches!(key.as_str(), "emoji" | "emjio" | "face" | "bq" | "biaoqing") {
        return emoji_panel_candidates_detailed("");
    }

    let mut matched = SYMBOL_ITEMS
        .iter()
        .filter(|item| symbol_item_matches(item, &key))
        .collect::<Vec<_>>();
    if matched.is_empty() {
        matched = SYMBOL_ITEMS
            .iter()
            .filter(|item| matches!(item.group, "punct" | "quote" | "bracket" | "math" | "arrow"))
            .collect();
    }

    let mut seen = HashSet::new();
    let mut out = Vec::new();
    for item in matched {
        if !seen.insert(item.symbol) {
            continue;
        }
        out.push(DirectCandidate {
            phrase: item.symbol.to_string(),
            score: 150.0 - out.len() as f64,
            meta: Some(symbol_meta(item)),
        });
    }
    out
}

fn symbol_panel_candidates(arg: &str) -> Vec<(String, f64)> {
    symbol_panel_candidates_detailed(arg)
        .into_iter()
        .map(|item| (item.phrase, item.score))
        .collect()
}

fn emoji_panel_candidates(arg: &str) -> Vec<(String, f64)> {
    emoji_panel_candidates_detailed(arg)
        .into_iter()
        .map(|item| (item.phrase, item.score))
        .collect()
}

fn unit_candidates(arg: &str) -> Vec<(String, f64)> {
    let key = arg.trim().to_ascii_lowercase();
    let texts = match key.as_str() {
        "" => vec![
            "℃ °C °F K",
            "mm cm m km",
            "g kg t mL L",
            "㎡ m² km² 立方米",
            "m/s km/h",
            "¥ $ € £",
        ],
        "temp" | "temperature" => vec!["℃ °C °F K", "摄氏度 华氏度 开尔文"],
        "length" | "distance" => vec!["mm cm m km", "μm nm", "英寸 英尺 码 英里"],
        "weight" | "mass" => vec!["mg g kg t", "斤 两 克 千克 吨"],
        "area" => vec!["㎡ m² km²", "平方厘米 平方米 平方公里", "亩 公顷 ha"],
        "speed" => vec!["m/s km/h", "米/秒 千米/时"],
        "money" | "currency" => vec!["¥ $ € £", "人民币 美元 欧元 英镑"],
        _ => vec!["℃ °C °F K", "mm cm m km", "g kg t mL L", "㎡ m² km²"],
    };
    score_candidates(texts.into_iter().map(str::to_string), 128.0)
}

fn formal_currency_candidates(arg: &str) -> Vec<(String, f64)> {
    let trimmed = arg.trim();
    if trimmed.is_empty() {
        return score_candidates(
            vec!["壹佰贰拾叁元肆角伍分", "壹仟零壹元整", "用法：vv dx 123.45"]
                .into_iter()
                .map(str::to_string),
            126.0,
        );
    }
    if let Some((upper, canonical)) = uppercase_currency(trimmed) {
        return score_candidates(vec![upper, format!("￥{canonical}"), canonical], 126.0);
    }
    score_candidates(
        vec![
            "用法：vv dx 123.45".to_string(),
            "支持纯数字或两位小数".to_string(),
        ],
        90.0,
    )
}

fn email_candidates(arg: &str) -> Vec<(String, f64)> {
    let trimmed = arg.trim();
    if trimmed.is_empty() {
        return score_candidates(
            vec![
                "name@example.com",
                "name@qq.com",
                "name@163.com",
                "name@gmail.com",
                "mailto:name@example.com",
            ]
            .into_iter()
            .map(str::to_string),
            124.0,
        );
    }
    if trimmed.contains('@') {
        return score_candidates(
            vec![trimmed.to_string(), format!("mailto:{trimmed}")],
            124.0,
        );
    }
    score_candidates(
        vec![
            format!("{trimmed}@qq.com"),
            format!("{trimmed}@163.com"),
            format!("{trimmed}@gmail.com"),
            format!("{trimmed}@outlook.com"),
            format!("{trimmed}@example.com"),
        ],
        124.0,
    )
}

fn url_candidates(cmd: &str, arg: &str) -> Vec<(String, f64)> {
    let trimmed = arg.trim();
    if trimmed.is_empty() {
        return score_candidates(
            vec![
                "https://example.com",
                "https://www.example.com",
                "http://example.com",
                "https://example.com/path",
            ]
            .into_iter()
            .map(str::to_string),
            124.0,
        );
    }
    if trimmed.contains("://") {
        return score_candidates(vec![trimmed.to_string()], 124.0);
    }

    let mut base = trimmed.trim_matches('/').to_string();
    let host = base.split('/').next().unwrap_or_default();
    if !host.contains('.') {
        base = if let Some((head, tail)) = base.split_once('/') {
            format!("{head}.com/{tail}")
        } else {
            format!("{base}.com")
        };
    }

    let mut texts = Vec::new();
    match cmd {
        "http" => texts.push(format!("http://{base}")),
        "https" => texts.push(format!("https://{base}")),
        _ => {
            texts.push(format!("https://{base}"));
            if !base.starts_with("www.") {
                texts.push(format!("https://www.{base}"));
            }
            texts.push(format!("http://{base}"));
        }
    }
    score_candidates(texts, 124.0)
}

fn markdown_candidates(arg: &str) -> Vec<(String, f64)> {
    let trimmed = arg.trim();
    if trimmed.is_empty() {
        return score_candidates(
            vec![
                "# 标题",
                "## 小节",
                "> 引用",
                "- [ ] 待办",
                "[文本](https://example.com)",
                "```text\n\n```",
                "| 列1 | 列2 |\n| --- | --- |\n| 内容 | 内容 |",
            ]
            .into_iter()
            .map(str::to_string),
            122.0,
        );
    }

    let (cmd, rest) = split_command_args(trimmed);
    let key = cmd.to_ascii_lowercase();
    let texts = match key.as_str() {
        "h1" => vec![format!("# {}", if rest.is_empty() { "标题" } else { rest })],
        "h2" => vec![format!(
            "## {}",
            if rest.is_empty() { "小节" } else { rest }
        )],
        "h3" => vec![format!(
            "### {}",
            if rest.is_empty() {
                "三级标题"
            } else {
                rest
            }
        )],
        "quote" => vec![format!("> {}", if rest.is_empty() { "引用" } else { rest })],
        "task" => vec![format!(
            "- [ ] {}",
            if rest.is_empty() { "待办" } else { rest }
        )],
        "code" => {
            let lang = if rest.is_empty() { "text" } else { rest };
            vec![format!("```{lang}\n\n```")]
        }
        "link" => {
            if let Some((label, url)) = rest.split_once('|') {
                vec![format!("[{}]({})", label.trim(), url.trim())]
            } else {
                vec!["[文本](https://example.com)".to_string()]
            }
        }
        "table" => vec!["| 列1 | 列2 |\n| --- | --- |\n| 内容 | 内容 |".to_string()],
        _ => vec![
            format!("# {trimmed}"),
            format!("## {trimmed}"),
            format!("- [ ] {trimmed}"),
        ],
    };
    score_candidates(texts, 122.0)
}

fn clipboard_help_candidates() -> Vec<(String, f64)> {
    score_candidates(
        vec![
            "vv cb - 最近文本剪贴板".to_string(),
            "vv cb pinned - 仅看置顶".to_string(),
            "vv cb recent - 仅看历史".to_string(),
            "vv cb pin - 置顶当前剪贴板".to_string(),
            "vv cb unpin - 取消置顶当前剪贴板".to_string(),
            "vv cb clear - 清空非置顶历史".to_string(),
            format!("vvu - 前 {CLIPBOARD_QUICK_LIMIT} 条剪贴板快粘"),
            "vvu type:url / vvu 7d - 按类型或时间过滤".to_string(),
            "vvu open - 打开独立剪贴板管理器".to_string(),
        ],
        118.0,
    )
}

fn clipboard_manager_path() -> Option<PathBuf> {
    runtime_helper_path("srf_ime_clipboard.exe")
}

fn handwrite_path() -> Option<PathBuf> {
    runtime_helper_path("srf_ime_handwrite.exe")
}

fn runtime_helper_path(exe_name: &str) -> Option<PathBuf> {
    let mut candidates = Vec::new();
    if let Ok(exe) = std::env::current_exe() {
        if let Some(dir) = exe.parent() {
            candidates.push(dir.join(exe_name));
        }
    }
    if let Ok(la) = std::env::var("LOCALAPPDATA") {
        candidates.push(
            PathBuf::from(la)
                .join("Programs")
                .join(crate::app_paths::APP_PATH_NAME)
                .join(exe_name),
        );
    }
    if let Ok(pf) = std::env::var("ProgramFiles") {
        candidates.push(
            PathBuf::from(pf)
                .join(crate::app_paths::APP_PATH_NAME)
                .join(exe_name),
        );
    }
    if let Ok(pf86) = std::env::var("ProgramFiles(x86)") {
        candidates.push(
            PathBuf::from(pf86)
                .join(crate::app_paths::APP_PATH_NAME)
                .join(exe_name),
        );
    }
    candidates.into_iter().find(|p| p.is_file())
}

fn open_clipboard_manager_candidates(initial_filter: &str) -> Vec<(String, f64)> {
    let Some(path) = clipboard_manager_path() else {
        return score_candidates(
            vec![
                "未找到 srf_ime_clipboard.exe".to_string(),
                "可从托盘菜单打开剪贴板管理器（安装后）".to_string(),
            ],
            200.0,
        );
    };
    let mut cmd = std::process::Command::new(&path);
    if !initial_filter.trim().is_empty() {
        cmd.arg("--search").arg(initial_filter.trim());
    }
    if let Some(dir) = path.parent() {
        cmd.current_dir(dir);
    }
    match cmd.spawn() {
        Ok(_) => score_candidates(
            vec![
                "已唤起剪贴板管理器".to_string(),
                "支持字母区上方 1-9 快速粘贴历史项".to_string(),
            ],
            220.0,
        ),
        Err(e) => score_candidates(
            vec![
                format!("唤起失败：{e}"),
                path.display().to_string(),
                "可尝试从托盘菜单再次打开".to_string(),
            ],
            150.0,
        ),
    }
}

fn clipboard_candidate_preview_text(text: &str) -> String {
    let mut lines = Vec::new();
    let mut truncated = false;

    for raw_line in text.replace("\r\n", "\n").replace('\r', "\n").lines() {
        let mut line = String::new();
        let mut pending_space = false;
        for ch in raw_line.trim().chars() {
            if ch.is_whitespace() {
                if !line.is_empty() {
                    pending_space = true;
                }
                continue;
            }
            if pending_space && !line.is_empty() {
                line.push(' ');
            }
            pending_space = false;
            if line.chars().count() >= CLIPBOARD_QUICK_PREVIEW_LINE_CHARS {
                truncated = true;
                break;
            }
            line.push(ch);
        }
        if !line.is_empty() {
            lines.push(line);
            if lines.len() >= CLIPBOARD_QUICK_PREVIEW_LINES {
                break;
            }
        }
    }
    if text
        .replace("\r\n", "\n")
        .replace('\r', "\n")
        .lines()
        .filter(|line| !line.trim().is_empty())
        .count()
        > lines.len()
    {
        truncated = true;
    }

    if lines.is_empty() {
        return String::new();
    }

    let mut preview = lines.join("\n");
    if preview.chars().count() > CLIPBOARD_QUICK_PREVIEW_CHARS * CLIPBOARD_QUICK_PREVIEW_LINES {
        preview = preview
            .chars()
            .take(CLIPBOARD_QUICK_PREVIEW_CHARS * CLIPBOARD_QUICK_PREVIEW_LINES - 1)
            .collect();
        truncated = true;
    }
    if truncated && !preview.is_empty() {
        preview.push('\u{2026}');
    }
    preview
}

fn clipboard_quick_token(id: &str) -> String {
    format!("clipboard://{id}")
}

fn clipboard_quick_phrase(entry: &clipboard_store::ClipboardEntry) -> String {
    if entry.text.encode_utf16().count() <= CLIPBOARD_QUICK_INLINE_TEXT_UTF16_LIMIT {
        entry.text.clone()
    } else {
        clipboard_quick_token(&entry.id)
    }
}

fn clipboard_display_meta(
    entry: &clipboard_store::ClipboardEntry,
    preview: &str,
    pinned: bool,
    page: usize,
    total_pages: usize,
) -> String {
    let mut meta = format!(
        "{CLIPBOARD_DISPLAY_META_PREFIX}{preview}\t{CLIPBOARD_NO_LEARN_META}\t{CLIPBOARD_QUICK_META}\t{CLIPBOARD_ID_META_PREFIX}{}\t{CLIPBOARD_VERTICAL_LAYOUT_META}\t{CLIPBOARD_PAGE_META_PREFIX}{}\t{CLIPBOARD_PAGES_META_PREFIX}{}",
        entry.id,
        page.saturating_add(1),
        total_pages.max(1),
    );
    if pinned {
        meta.push('\t');
        meta.push_str(CLIPBOARD_PINNED_META);
    }
    meta
}

fn parse_clipboard_quick_arg(arg: &str) -> (usize, &str) {
    let trimmed = arg.trim();
    if trimmed.is_empty() {
        return (0, "");
    }
    let mut parts = trimmed.splitn(2, char::is_whitespace);
    let first = parts.next().unwrap_or("");
    let rest = parts.next().unwrap_or("").trim();
    if first.starts_with(['p', 'P']) {
        if let Some(page) = parse_clipboard_quick_page(first) {
            return (page, rest);
        }
    }
    (0, trimmed)
}

fn parse_u_clipboard_quick_command<'a>(cmd: &str, arg: &'a str) -> Option<(usize, &'a str)> {
    if cmd.eq_ignore_ascii_case("u") {
        return Some(parse_clipboard_quick_arg(arg));
    }
    let tail = cmd.strip_prefix('u').or_else(|| cmd.strip_prefix('U'))?;
    if tail.is_empty() || !tail.chars().all(|ch| ch.is_ascii_digit()) {
        return None;
    }
    Some((parse_clipboard_quick_page(tail).unwrap_or(0), arg.trim()))
}

pub fn is_clipboard_command(rest: &str) -> bool {
    let t = rest.trim();
    if t.is_empty() {
        return false;
    }
    let (cmd, arg) = split_command_args(t);
    parse_u_clipboard_quick_command(cmd, arg).is_some()
        || command_matches(&cmd.to_ascii_lowercase(), "clipboard")
}

fn parse_clipboard_quick_page(token: &str) -> Option<usize> {
    let token = token.trim();
    let raw = token
        .strip_prefix('p')
        .or_else(|| token.strip_prefix('P'))
        .unwrap_or(token);
    let page = raw.parse::<usize>().ok()?;
    if page == 0 {
        None
    } else {
        Some(page - 1)
    }
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

fn parse_days_filter(value: &str) -> Option<u64> {
    let lower = value.trim().to_ascii_lowercase();
    if matches!(lower.as_str(), "today" | "今天") {
        return Some(1);
    }
    lower
        .strip_suffix('d')
        .or_else(|| lower.strip_suffix("day"))
        .or_else(|| lower.strip_suffix("days"))
        .or_else(|| lower.strip_suffix("天"))
        .and_then(|days| days.parse::<u64>().ok())
        .filter(|days| *days > 0)
}

fn clipboard_looks_like_url(text: &str) -> bool {
    let lower = text.trim().to_ascii_lowercase();
    lower.starts_with("http://")
        || lower.starts_with("https://")
        || lower.starts_with("www.")
        || lower.contains("://")
}

fn clipboard_looks_like_email(text: &str) -> bool {
    let lower = text.trim().to_ascii_lowercase();
    lower.contains('@')
        && lower
            .split('@')
            .nth(1)
            .is_some_and(|tail| tail.contains('.'))
}

fn clipboard_looks_like_path(text: &str) -> bool {
    let trimmed = text.trim();
    let bytes = trimmed.as_bytes();
    bytes.get(1) == Some(&b':')
        || trimmed.starts_with(r"\\")
        || trimmed.starts_with('/')
        || trimmed.contains('\\')
}

fn clipboard_looks_like_code(text: &str) -> bool {
    let trimmed = text.trim();
    if trimmed.len() < 3 {
        return false;
    }

    let strong_patterns = [
        "```",
        "=>",
        "::",
        "fn ",
        "function ",
        "class ",
        "def ",
        "import ",
        "#include",
        "#!",
        "pub fn",
        "impl ",
        "match ",
        "struct ",
        "enum ",
        "trait ",
        "const ",
        "let ",
        "var ",
        "console.log",
        "print(",
        "printf(",
        "std::",
        "async ",
        "await ",
        "=> ",
        "-> ",
    ];

    let mut score = 0i32;
    for pattern in strong_patterns {
        if trimmed.contains(pattern) {
            score += 2;
        }
    }
    let has_brace_pair = trimmed.contains('{') && trimmed.contains('}');
    let has_semi_equal = trimmed.contains(';') && trimmed.contains('=');
    if has_brace_pair {
        score += 1;
    }
    if has_semi_equal {
        score += 1;
    }
    for pattern in [
        " if ", " for ", " while ", " return", " else ", "//", "/*", "\t", "\n  ", "    ",
    ] {
        if trimmed.contains(pattern) {
            score += 1;
        }
    }

    score >= 2
}

fn clipboard_type_matches(entry: &clipboard_store::ClipboardEntry, value: &str) -> bool {
    match value.trim().to_ascii_lowercase().as_str() {
        "url" | "site" | "网址" => clipboard_looks_like_url(&entry.text),
        "email" | "mail" | "邮箱" => clipboard_looks_like_email(&entry.text),
        "path" | "file" | "路径" => clipboard_looks_like_path(&entry.text),
        "code" | "代码" => clipboard_looks_like_code(&entry.text),
        "text" | "文本" => {
            !clipboard_looks_like_url(&entry.text)
                && !clipboard_looks_like_email(&entry.text)
                && !clipboard_looks_like_path(&entry.text)
                && !clipboard_looks_like_code(&entry.text)
        }
        _ => false,
    }
}

fn clipboard_entry_matches_filter(entry: &clipboard_store::ClipboardEntry, filter: &str) -> bool {
    let now = now_secs();
    let mut matched_text_token = false;
    for token in filter.split_whitespace() {
        let lower = token.to_ascii_lowercase();
        if let Some(days) = parse_days_filter(&lower) {
            let cutoff = now.saturating_sub(days.saturating_mul(86_400));
            if entry.captured_at < cutoff {
                return false;
            }
            continue;
        }
        if let Some(kind) = lower
            .strip_prefix("type:")
            .or_else(|| lower.strip_prefix("kind:"))
            .or_else(|| lower.strip_prefix("类型:"))
        {
            if !clipboard_type_matches(entry, kind) {
                return false;
            }
            continue;
        }
        matched_text_token = true;
        if !entry.text.to_ascii_lowercase().contains(&lower) && !entry.text.contains(token) {
            return false;
        }
    }
    !matched_text_token || !entry.text.trim().is_empty()
}

fn quick_clipboard_candidates_from_snapshot(
    snapshot: &clipboard_store::ClipboardSnapshot,
    page: usize,
    filter: &str,
) -> Vec<DirectCandidate> {
    let mut seen = HashSet::new();
    let mut ordered = Vec::new();
    for entry in snapshot.history.iter() {
        let text = entry.text.as_str();
        if text.trim().is_empty()
            || !clipboard_entry_matches_filter(entry, filter)
            || !seen.insert(text.to_string())
        {
            continue;
        }
        ordered.push(entry);
    }

    seen.clear();
    let pinned_ids = snapshot
        .pinned
        .iter()
        .map(|entry| entry.id.as_str())
        .collect::<HashSet<_>>();
    let mut pinned_first = Vec::new();
    for entry in snapshot.pinned.iter() {
        let text = entry.text.as_str();
        if text.trim().is_empty()
            || !clipboard_entry_matches_filter(entry, filter)
            || !seen.insert(text.to_string())
        {
            continue;
        }
        pinned_first.push(entry);
        if pinned_first.len() >= CLIPBOARD_QUICK_PINNED_LIMIT {
            break;
        }
    }
    for entry in &pinned_first {
        ordered.retain(|history| history.text != entry.text);
    }
    pinned_first.append(&mut ordered);

    let total_pages = ordered
        .len()
        .saturating_add(pinned_first.len())
        .saturating_add(CLIPBOARD_QUICK_LIMIT - 1)
        / CLIPBOARD_QUICK_LIMIT;
    let offset = page.saturating_mul(CLIPBOARD_QUICK_LIMIT);
    let mut out = Vec::new();
    for entry in pinned_first
        .into_iter()
        .skip(offset)
        .take(CLIPBOARD_QUICK_LIMIT)
    {
        if entry.text.trim().is_empty() {
            continue;
        }
        let preview = clipboard_candidate_preview_text(entry.text.as_str());
        out.push(DirectCandidate {
            phrase: clipboard_quick_phrase(entry),
            score: 220.0 - out.len() as f64,
            meta: Some(clipboard_display_meta(
                entry,
                &preview,
                pinned_ids.contains(entry.id.as_str()),
                page,
                total_pages,
            )),
        });
    }
    out
}

fn quick_clipboard_candidates_or_help(
    snapshot: &clipboard_store::ClipboardSnapshot,
    page: usize,
    filter: &str,
) -> Vec<DirectCandidate> {
    let out = quick_clipboard_candidates_from_snapshot(snapshot, page, filter);
    if out.is_empty() {
        scored_to_direct(score_candidates(
            vec![
                "剪贴板暂无文本历史".to_string(),
                "复制一段文字后再输入 vvu 可快粘".to_string(),
                "vvu open - 打开剪贴板管理器".to_string(),
            ],
            118.0,
        ))
    } else {
        out
    }
}

fn open_handwrite_candidates() -> Vec<(String, f64)> {
    let Some(path) = handwrite_path() else {
        return score_candidates(
            vec![
                "未找到 srf_ime_handwrite.exe".to_string(),
                "可从托盘菜单或设置页打开手写查字（安装后）".to_string(),
            ],
            200.0,
        );
    };
    let mut cmd = std::process::Command::new(&path);
    if let Some(dir) = path.parent() {
        cmd.current_dir(dir);
    }
    match cmd.spawn() {
        Ok(_) => score_candidates(
            vec![
                "已唤起手写查字窗口".to_string(),
                "画完一个字后可复制或粘贴候选".to_string(),
            ],
            220.0,
        ),
        Err(e) => score_candidates(
            vec![
                format!("唤起失败：{e}"),
                path.display().to_string(),
                "可尝试从托盘菜单再次打开".to_string(),
            ],
            150.0,
        ),
    }
}

fn clipboard_text_candidates(
    snapshot: &clipboard_store::ClipboardSnapshot,
    include_pinned: bool,
    include_history: bool,
    filter: &str,
) -> Vec<(String, f64)> {
    let mut texts = Vec::new();
    if include_pinned {
        texts.extend(
            snapshot
                .pinned
                .iter()
                .filter(|entry| clipboard_entry_matches_filter(entry, filter))
                .map(|entry| entry.text.clone()),
        );
    }
    if include_history {
        texts.extend(
            snapshot
                .history
                .iter()
                .filter(|entry| clipboard_entry_matches_filter(entry, filter))
                .map(|entry| entry.text.clone()),
        );
    }

    let out = score_candidates(texts, 118.0);
    if out.is_empty() {
        clipboard_help_candidates()
    } else {
        out
    }
}

fn clipboard_text_direct_candidates(
    snapshot: &clipboard_store::ClipboardSnapshot,
    include_pinned: bool,
    include_history: bool,
    filter: &str,
) -> Vec<DirectCandidate> {
    let mut entries = Vec::new();
    let mut seen = HashSet::new();
    if include_pinned {
        for entry in snapshot
            .pinned
            .iter()
            .filter(|entry| clipboard_entry_matches_filter(entry, filter))
        {
            if seen.insert(entry.text.clone()) {
                entries.push(entry);
            }
        }
    }
    if include_history {
        for entry in snapshot
            .history
            .iter()
            .filter(|entry| clipboard_entry_matches_filter(entry, filter))
        {
            if seen.insert(entry.text.clone()) {
                entries.push(entry);
            }
        }
    }

    let pinned_ids = snapshot
        .pinned
        .iter()
        .map(|entry| entry.id.as_str())
        .collect::<HashSet<_>>();
    let mut out = Vec::new();
    for (idx, entry) in entries.into_iter().enumerate() {
        if entry.text.trim().is_empty() {
            continue;
        }
        let preview = if clipboard_store::clipboard_candidate_preview_enabled() {
            clipboard_candidate_preview_text(&entry.text)
        } else {
            "剪贴板".to_string()
        };
        out.push(DirectCandidate {
            phrase: clipboard_quick_token(&entry.id),
            score: 118.0 - idx as f64,
            meta: Some(clipboard_display_meta(
                entry,
                &preview,
                pinned_ids.contains(entry.id.as_str()),
                0,
                1,
            )),
        });
    }
    if out.is_empty() {
        scored_to_direct(clipboard_help_candidates())
    } else {
        out
    }
}

fn clipboard_candidates(arg: &str) -> Vec<(String, f64)> {
    let trimmed = arg.trim();
    let current_snapshot = clipboard_store::capture_system_clipboard_snapshot()
        .unwrap_or_else(|_| clipboard_store::cached_snapshot().unwrap_or_default());

    let (cmd, rest) = split_command_args(trimmed);
    let key = cmd.to_ascii_lowercase();
    match key.as_str() {
        "" => clipboard_text_candidates(&current_snapshot, true, true, ""),
        "pin" => {
            let _ = clipboard_store::pin_current_clipboard();
            let snapshot = clipboard_store::snapshot().unwrap_or_default();
            clipboard_text_candidates(&snapshot, true, false, "")
        }
        "unpin" => {
            let _ = clipboard_store::unpin_current_clipboard();
            let snapshot = clipboard_store::snapshot().unwrap_or_default();
            clipboard_text_candidates(&snapshot, true, true, "")
        }
        "pinned" | "top" | "saved" => {
            clipboard_text_candidates(&current_snapshot, true, false, rest)
        }
        "recent" | "history" | "hist" => {
            clipboard_text_candidates(&current_snapshot, false, true, rest)
        }
        "clear" => {
            let _ = clipboard_store::clear_history();
            let snapshot = clipboard_store::snapshot().unwrap_or_default();
            clipboard_text_candidates(&snapshot, true, false, "")
        }
        "clearall" => {
            let _ = clipboard_store::clear_all();
            clipboard_help_candidates()
        }
        "refresh" => {
            let _ = clipboard_store::capture_system_clipboard(true);
            let snapshot = clipboard_store::snapshot().unwrap_or_default();
            clipboard_text_candidates(&snapshot, true, true, "")
        }
        "open" | "manager" | "mgr" | "window" => open_clipboard_manager_candidates(rest),
        _ => clipboard_text_candidates(&current_snapshot, true, true, trimmed),
    }
}

fn clipboard_candidates_detailed(arg: &str) -> Vec<DirectCandidate> {
    let trimmed = arg.trim();
    let current_snapshot = clipboard_store::capture_system_clipboard_snapshot()
        .unwrap_or_else(|_| clipboard_store::cached_snapshot().unwrap_or_default());

    let (cmd, rest) = split_command_args(trimmed);
    let key = cmd.to_ascii_lowercase();
    match key.as_str() {
        "" => clipboard_text_direct_candidates(&current_snapshot, true, true, ""),
        "pin" => {
            let _ = clipboard_store::pin_current_clipboard();
            let snapshot = clipboard_store::snapshot().unwrap_or_default();
            clipboard_text_direct_candidates(&snapshot, true, false, "")
        }
        "unpin" => {
            let _ = clipboard_store::unpin_current_clipboard();
            let snapshot = clipboard_store::snapshot().unwrap_or_default();
            clipboard_text_direct_candidates(&snapshot, true, true, "")
        }
        "pinned" | "top" | "saved" => {
            clipboard_text_direct_candidates(&current_snapshot, true, false, rest)
        }
        "recent" | "history" | "hist" => {
            clipboard_text_direct_candidates(&current_snapshot, false, true, rest)
        }
        "clear" => {
            let _ = clipboard_store::clear_history();
            let snapshot = clipboard_store::snapshot().unwrap_or_default();
            clipboard_text_direct_candidates(&snapshot, true, false, "")
        }
        "clearall" => {
            let _ = clipboard_store::clear_all();
            scored_to_direct(clipboard_help_candidates())
        }
        "refresh" => {
            let _ = clipboard_store::capture_system_clipboard(true);
            let snapshot = clipboard_store::snapshot().unwrap_or_default();
            clipboard_text_direct_candidates(&snapshot, true, true, "")
        }
        "open" | "manager" | "mgr" | "window" => {
            scored_to_direct(open_clipboard_manager_candidates(rest))
        }
        _ => clipboard_text_direct_candidates(&current_snapshot, true, true, trimmed),
    }
}

pub fn lookup_detailed(rest: &str) -> Option<Vec<DirectCandidate>> {
    lookup_detailed_with_options(rest, LookupOptions::default())
}

pub fn lookup_detailed_with_options(
    rest: &str,
    options: LookupOptions,
) -> Option<Vec<DirectCandidate>> {
    let t = rest.trim();
    if let Some(custom) = crate::custom_shortcuts::lookup_detailed(t) {
        return Some(
            custom
                .into_iter()
                .enumerate()
                .map(|(idx, item)| DirectCandidate {
                    phrase: item.phrase,
                    score: 10_000.0 - idx as f64,
                    meta: Some(format!("自定义: {}", item.source_key)),
                })
                .collect(),
        );
    }
    if let Some(items) = crate::v_tools::lookup(t) {
        return Some(
            items
                .into_iter()
                .enumerate()
                .map(|(idx, item)| DirectCandidate {
                    phrase: item.phrase,
                    score: DIRECT_SHORTCUT_TOP_SCORE - idx as f64,
                    meta: Some(format!("{}\t{}", item.meta, DIRECT_NO_LEARN_META)),
                })
                .collect(),
        );
    }
    if !t.is_empty() {
        let (cmd, arg) = split_command_args(t);
        if let Some((page, filter)) = parse_u_clipboard_quick_command(cmd, arg) {
            let (quick_cmd, quick_rest) = split_command_args(arg.trim());
            let quick_cmd = quick_cmd.to_ascii_lowercase();
            if page == 0 && matches!(quick_cmd.as_str(), "open" | "manager" | "mgr" | "window") {
                return Some(scored_to_direct(open_clipboard_manager_candidates(
                    quick_rest,
                )));
            }
            let snapshot = clipboard_store::capture_system_clipboard_snapshot()
                .unwrap_or_else(|_| clipboard_store::cached_snapshot().unwrap_or_default());
            return Some(quick_clipboard_candidates_or_help(&snapshot, page, filter));
        }
        let cmd_lower = cmd.to_ascii_lowercase();
        if command_matches(&cmd_lower, "clipboard") {
            return Some(clipboard_candidates_detailed(arg));
        }
        if command_matches(&cmd_lower, "symbol") {
            if !options.symbol_toolbox_enabled {
                return Some(Vec::new());
            }
            if matches!(
                arg.trim().to_ascii_lowercase().as_str(),
                "emoji" | "emjio" | "face" | "bq" | "biaoqing"
            ) && !options.emoji_input_enabled
            {
                return Some(Vec::new());
            }
            return Some(symbol_panel_candidates_detailed(arg));
        }
        if command_matches(&cmd_lower, "emoji") {
            if !options.emoji_input_enabled {
                return Some(Vec::new());
            }
            return Some(emoji_panel_candidates_detailed(arg));
        }
    }
    lookup_with_options(t, options).map(scored_to_direct)
}

pub fn lookup(rest: &str) -> Option<Vec<(String, f64)>> {
    lookup_with_options(rest, LookupOptions::default())
}

pub fn lookup_with_options(rest: &str, options: LookupOptions) -> Option<Vec<(String, f64)>> {
    let t = rest.trim();
    if t.is_empty() {
        let mut help = vec![
            "vv rq - 北京日期时间".to_string(),
            "vv sj - 北京时间".to_string(),
            "vv xq - 星期/周几".to_string(),
            "vv dx 123.45 - 人民币大写".to_string(),
        ];
        if options.symbol_toolbox_enabled {
            help.push("vv sym - 常用符号".to_string());
        }
        if options.emoji_input_enabled {
            help.push("vv emoji - Emoji 表情".to_string());
        }
        help.extend([
            "vv rq mingtian / vv rq +3 - 相对日期".to_string(),
            "vv num 12345 / vv full abc123 - 数字格式".to_string(),
            "vv calc 23*17 - 计算器/单位换算".to_string(),
            "vv unit - 常用单位".to_string(),
            "vv mail alice - 邮箱模板".to_string(),
            "vv url openai - 网址模板".to_string(),
            "vv md h1 标题 - Markdown 片段".to_string(),
            "vv cb - 文本剪贴板历史/置顶".to_string(),
            "vv hw - 打开手写查字".to_string(),
            format!("vvu - 剪贴板前 {CLIPBOARD_QUICK_LIMIT} 条快粘"),
            "vv 数字 - 中文读法".to_string(),
        ]);
        return Some(score_candidates(help, 100.0));
    }

    if let Some(custom) = crate::custom_shortcuts::lookup(t) {
        return Some(custom);
    }

    if let Some(items) = crate::v_tools::lookup(t) {
        return Some(
            items
                .into_iter()
                .enumerate()
                .map(|(idx, item)| (item.phrase, DIRECT_SHORTCUT_TOP_SCORE - idx as f64))
                .collect(),
        );
    }

    let low = t.to_ascii_lowercase();
    if let Some(out) = function_key_candidates(low.as_str()) {
        return Some(
            out.into_iter()
                .map(|item| (item.phrase, item.score))
                .collect(),
        );
    }
    if let Some(out) = datetime_candidates_for(low.as_str()) {
        return Some(
            out.into_iter()
                .map(|item| (item.phrase, item.score))
                .collect(),
        );
    }

    let (cmd, arg) = split_command_args(t);
    if let Some((page, filter)) = parse_u_clipboard_quick_command(cmd, arg) {
        let (quick_cmd, quick_rest) = split_command_args(arg.trim());
        let quick_cmd = quick_cmd.to_ascii_lowercase();
        if page == 0 && matches!(quick_cmd.as_str(), "open" | "manager" | "mgr" | "window") {
            return Some(open_clipboard_manager_candidates(quick_rest));
        }
        let snapshot = clipboard_store::capture_system_clipboard_snapshot()
            .unwrap_or_else(|_| clipboard_store::cached_snapshot().unwrap_or_default());
        return Some(
            quick_clipboard_candidates_or_help(&snapshot, page, filter)
                .into_iter()
                .map(|item| (item.phrase, item.score))
                .collect(),
        );
    }
    let cmd_lower = cmd.to_ascii_lowercase();
    if command_matches(&cmd_lower, "week") {
        return Some(weekday_candidates());
    }
    if command_matches(&cmd_lower, "symbol") {
        if !options.symbol_toolbox_enabled {
            return Some(Vec::new());
        }
        if matches!(
            arg.trim().to_ascii_lowercase().as_str(),
            "emoji" | "emjio" | "face" | "bq" | "biaoqing"
        ) && !options.emoji_input_enabled
        {
            return Some(Vec::new());
        }
        return Some(symbol_panel_candidates(arg));
    }
    if command_matches(&cmd_lower, "emoji") {
        if !options.emoji_input_enabled {
            return Some(Vec::new());
        }
        return Some(emoji_panel_candidates(arg));
    }
    if command_matches(&cmd_lower, "unit") {
        return Some(unit_candidates(arg));
    }
    if command_matches(&cmd_lower, "currency") {
        return Some(formal_currency_candidates(arg));
    }
    if command_matches(&cmd_lower, "email") {
        return Some(email_candidates(arg));
    }
    if command_matches(&cmd_lower, "url") {
        return Some(url_candidates(cmd_lower.as_str(), arg));
    }
    if command_matches(&cmd_lower, "markdown") {
        return Some(markdown_candidates(arg));
    }
    if command_matches(&cmd_lower, "clipboard") {
        return Some(clipboard_candidates(arg));
    }
    if command_matches(&cmd_lower, "handwrite") {
        return Some(open_handwrite_candidates());
    }

    if let Some(out) = numeric_candidates(t) {
        return Some(out);
    }

    if t.len() <= 12 && t.chars().all(|c| c.is_ascii_alphabetic()) {
        let mut help = vec![
            "vv rq / vv sj / vv xq".to_string(),
            "vv cb - 剪贴板历史".to_string(),
            "vv hw - 手写查字".to_string(),
        ];
        if options.symbol_toolbox_enabled {
            help.push("vv sym - 常用符号".to_string());
        }
        if options.emoji_input_enabled {
            help.push("vv emoji - Emoji 表情".to_string());
        }
        help.push("vv md - Markdown 片段".to_string());
        return Some(score_candidates(help, 50.0));
    }

    None
}
