//! VV 直输助手中的日期计算、数字格式转换和轻量计算器。

use chrono::{Datelike, Duration, FixedOffset, NaiveDate, Utc, Weekday};

const BEIJING_UTC_OFFSET_SECS: i32 = 8 * 60 * 60;
const MAX_INPUT_CHARS: usize = 128;

#[derive(Clone, Debug, PartialEq)]
pub struct UtilityCandidate {
    pub phrase: String,
    pub meta: &'static str,
}

impl UtilityCandidate {
    fn new(phrase: impl Into<String>, meta: &'static str) -> Self {
        Self {
            phrase: phrase.into(),
            meta,
        }
    }
}

pub fn lookup(input: &str) -> Option<Vec<UtilityCandidate>> {
    let trimmed = input.trim();
    let (cmd, arg) = split_command_args(trimmed);
    let cmd = cmd.to_ascii_lowercase();

    if is_date_command(&cmd) && !arg.is_empty() {
        return Some(relative_date_candidates(arg));
    }
    if matches!(cmd.as_str(), "num" | "number" | "sz" | "shuzi") {
        return Some(number_candidates(arg));
    }
    if matches!(cmd.as_str(), "upper" | "daxie" | "cnupper") {
        return Some(uppercase_number_candidates(arg));
    }
    if matches!(cmd.as_str(), "full" | "fullwidth" | "qj" | "quanjiao") {
        return Some(fullwidth_candidates(arg));
    }
    if matches!(cmd.as_str(), "roman" | "lm" | "luoma") {
        return Some(roman_candidates(arg));
    }
    if matches!(cmd.as_str(), "hex" | "base" | "jinzhi") {
        return Some(radix_candidates(arg));
    }
    if matches!(cmd.as_str(), "percent" | "pct" | "baifenbi") {
        return Some(percent_candidates(arg));
    }
    if matches!(cmd.as_str(), "bytes" | "byte" | "size" | "daxiao") {
        return Some(byte_candidates(arg));
    }
    if matches!(cmd.as_str(), "calc" | "js" | "jisuan") {
        return Some(calculator_candidates(arg));
    }
    if matches!(cmd.as_str(), "convert" | "conv" | "to" | "huansuan") {
        return Some(unit_conversion_candidates(arg));
    }
    None
}

fn split_command_args(input: &str) -> (&str, &str) {
    if let Some(idx) = input.find(char::is_whitespace) {
        (&input[..idx], input[idx..].trim())
    } else {
        (input, "")
    }
}

fn is_date_command(cmd: &str) -> bool {
    matches!(
        cmd,
        "rq" | "date" | "jr" | "cal" | "calendar" | "rili" | "riqi"
    )
}

fn beijing_today() -> NaiveDate {
    let offset = FixedOffset::east_opt(BEIJING_UTC_OFFSET_SECS).expect("valid Beijing UTC offset");
    Utc::now().with_timezone(&offset).date_naive()
}

fn relative_date_candidates(arg: &str) -> Vec<UtilityCandidate> {
    relative_date_candidates_from(beijing_today(), arg).unwrap_or_default()
}

fn relative_date_candidates_from(today: NaiveDate, arg: &str) -> Option<Vec<UtilityCandidate>> {
    let key = arg.trim().to_ascii_lowercase().replace([' ', '_', '-'], "");
    let target = match key.as_str() {
        "today" | "jintian" | "jt" | "0" | "+0" => today,
        "tomorrow" | "mingtian" | "mt" | "+1" => today.checked_add_days(chrono::Days::new(1))?,
        "houtian" | "dayaftertomorrow" | "ht" | "+2" => {
            today.checked_add_days(chrono::Days::new(2))?
        }
        "yesterday" | "zuotian" | "zt" | "1" => today.checked_sub_days(chrono::Days::new(1))?,
        "qiantian" | "daybeforeyesterday" | "qt" | "2" => {
            today.checked_sub_days(chrono::Days::new(2))?
        }
        "monthstart" | "benyuechuyi" | "bycy" => {
            NaiveDate::from_ymd_opt(today.year(), today.month(), 1)?
        }
        "monthend" | "benyuemori" | "bymr" => last_day_of_month(today.year(), today.month())?,
        "nextmonthstart" | "xiayuechuyi" | "xycy" => {
            let (year, month) = next_month(today.year(), today.month());
            NaiveDate::from_ymd_opt(year, month, 1)?
        }
        "nextmonthend" | "xiayuemori" | "xymr" => {
            let (year, month) = next_month(today.year(), today.month());
            last_day_of_month(year, month)?
        }
        _ => {
            if let Some(days) = parse_signed_days(arg) {
                today.checked_add_signed(Duration::days(days))?
            } else if let Some(weekday) = parse_weekday(&key, "xiazhou") {
                date_in_next_week(today, weekday)?
            } else if let Some(weekday) = parse_weekday(&key, "next") {
                date_in_next_week(today, weekday)?
            } else if let Some(weekday) = parse_weekday(&key, "zhou") {
                next_weekday(today, weekday)?
            } else {
                return None;
            }
        }
    };

    Some(date_output_candidates(target))
}

fn parse_signed_days(input: &str) -> Option<i64> {
    let compact = input.trim().to_ascii_lowercase().replace(' ', "");
    let number = compact
        .strip_suffix("days")
        .or_else(|| compact.strip_suffix("day"))
        .or_else(|| compact.strip_suffix("tian"))
        .unwrap_or(&compact);
    if !number.starts_with(['+', '-']) {
        return None;
    }
    let value = number.parse::<i64>().ok()?;
    (value.unsigned_abs() <= 36_500).then_some(value)
}

fn parse_weekday(key: &str, prefix: &str) -> Option<Weekday> {
    let suffix = key.strip_prefix(prefix)?;
    match suffix {
        "yi" | "1" | "mon" | "monday" => Some(Weekday::Mon),
        "er" | "2" | "tue" | "tuesday" => Some(Weekday::Tue),
        "san" | "3" | "wed" | "wednesday" => Some(Weekday::Wed),
        "si" | "4" | "thu" | "thursday" => Some(Weekday::Thu),
        "wu" | "5" | "fri" | "friday" => Some(Weekday::Fri),
        "liu" | "6" | "sat" | "saturday" => Some(Weekday::Sat),
        "ri" | "tian" | "7" | "sun" | "sunday" => Some(Weekday::Sun),
        _ => None,
    }
}

fn date_in_next_week(today: NaiveDate, weekday: Weekday) -> Option<NaiveDate> {
    let current = today.weekday().num_days_from_monday() as i64;
    let target = weekday.num_days_from_monday() as i64;
    today.checked_add_signed(Duration::days(7 - current + target))
}

fn next_weekday(today: NaiveDate, weekday: Weekday) -> Option<NaiveDate> {
    let current = today.weekday().num_days_from_monday() as i64;
    let target = weekday.num_days_from_monday() as i64;
    let days = (target - current).rem_euclid(7);
    today.checked_add_signed(Duration::days(if days == 0 { 7 } else { days }))
}

fn next_month(year: i32, month: u32) -> (i32, u32) {
    if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    }
}

fn last_day_of_month(year: i32, month: u32) -> Option<NaiveDate> {
    let (next_year, next_month) = next_month(year, month);
    NaiveDate::from_ymd_opt(next_year, next_month, 1)?.pred_opt()
}

fn weekday_cn(date: NaiveDate) -> &'static str {
    match date.weekday() {
        Weekday::Mon => "星期一",
        Weekday::Tue => "星期二",
        Weekday::Wed => "星期三",
        Weekday::Thu => "星期四",
        Weekday::Fri => "星期五",
        Weekday::Sat => "星期六",
        Weekday::Sun => "星期日",
    }
}

fn date_output_candidates(date: NaiveDate) -> Vec<UtilityCandidate> {
    let iso = date.format("%Y-%m-%d").to_string();
    let chinese = format!("{}年{}月{}日", date.year(), date.month(), date.day());
    let compact = date.format("%Y%m%d").to_string();
    vec![
        UtilityCandidate::new(format!("{chinese}（{}）", weekday_cn(date)), "日期计算"),
        UtilityCandidate::new(iso, "ISO 日期"),
        UtilityCandidate::new(chinese, "中文日期"),
        UtilityCandidate::new(compact, "紧凑日期"),
    ]
}

fn clean_integer(input: &str) -> Option<(bool, String)> {
    let compact = input.trim().replace([',', '_', ' '], "");
    let (negative, digits) = compact
        .strip_prefix('-')
        .map(|rest| (true, rest))
        .unwrap_or((false, compact.as_str()));
    if digits.is_empty() || digits.len() > 16 || !digits.chars().all(|c| c.is_ascii_digit()) {
        return None;
    }
    let normalized = digits.trim_start_matches('0');
    Some((
        negative,
        if normalized.is_empty() {
            "0".to_string()
        } else {
            normalized.to_string()
        },
    ))
}

fn chinese_integer(input: &str, uppercase: bool) -> Option<String> {
    let (negative, digits) = clean_integer(input)?;
    let value = digits.parse::<u64>().ok()?;
    let digit_names = if uppercase {
        ["零", "壹", "贰", "叁", "肆", "伍", "陆", "柒", "捌", "玖"]
    } else {
        ["零", "一", "二", "三", "四", "五", "六", "七", "八", "九"]
    };
    let small_units = if uppercase {
        ["", "拾", "佰", "仟"]
    } else {
        ["", "十", "百", "千"]
    };
    let big_units = ["", "万", "亿", "兆"];
    let mut result = if value == 0 {
        digit_names[0].to_string()
    } else {
        let mut groups = Vec::new();
        let mut rest = value;
        while rest > 0 {
            groups.push((rest % 10_000) as u16);
            rest /= 10_000;
        }
        let mut out = String::new();
        let mut pending_zero = false;
        for idx in (0..groups.len()).rev() {
            let group = groups[idx];
            if group == 0 {
                pending_zero = !out.is_empty();
                continue;
            }
            if pending_zero || (!out.is_empty() && group < 1000) {
                out.push_str(digit_names[0]);
            }
            out.push_str(&chinese_group(group, &digit_names, &small_units));
            out.push_str(big_units.get(idx).copied().unwrap_or(""));
            pending_zero = false;
        }
        out
    };
    if !uppercase && result.starts_with("一十") {
        result = result.replacen("一十", "十", 1);
    }
    if negative && value != 0 {
        result.insert(0, '负');
    }
    Some(result)
}

fn chinese_group(group: u16, digits: &[&str; 10], units: &[&str; 4]) -> String {
    let values = [
        (group / 1000) % 10,
        (group / 100) % 10,
        (group / 10) % 10,
        group % 10,
    ];
    let mut out = String::new();
    let mut pending_zero = false;
    for (idx, digit) in values.into_iter().enumerate() {
        if digit == 0 {
            pending_zero = !out.is_empty();
        } else {
            if pending_zero {
                out.push_str(digits[0]);
            }
            out.push_str(digits[digit as usize]);
            out.push_str(units[3 - idx]);
            pending_zero = false;
        }
    }
    out
}

fn number_candidates(arg: &str) -> Vec<UtilityCandidate> {
    let Some(lower) = chinese_integer(arg, false) else {
        return Vec::new();
    };
    let Some(upper) = chinese_integer(arg, true) else {
        return Vec::new();
    };
    vec![
        UtilityCandidate::new(lower, "中文数字"),
        UtilityCandidate::new(upper, "大写数字"),
    ]
}

fn uppercase_number_candidates(arg: &str) -> Vec<UtilityCandidate> {
    chinese_integer(arg, true)
        .map(|value| vec![UtilityCandidate::new(value, "大写数字")])
        .unwrap_or_default()
}

fn fullwidth_candidates(arg: &str) -> Vec<UtilityCandidate> {
    if arg.is_empty() {
        return Vec::new();
    }
    let converted: String = arg
        .chars()
        .map(|ch| match ch {
            ' ' => '\u{3000}',
            '!'..='~' => char::from_u32(ch as u32 + 0xfee0).unwrap_or(ch),
            _ => ch,
        })
        .collect();
    vec![UtilityCandidate::new(converted, "全角字符")]
}

fn roman_candidates(arg: &str) -> Vec<UtilityCandidate> {
    let Ok(value) = arg.trim().parse::<u16>() else {
        return Vec::new();
    };
    if !(1..=3999).contains(&value) {
        return Vec::new();
    }
    let mut rest = value;
    let mut out = String::new();
    for (number, symbol) in [
        (1000, "M"),
        (900, "CM"),
        (500, "D"),
        (400, "CD"),
        (100, "C"),
        (90, "XC"),
        (50, "L"),
        (40, "XL"),
        (10, "X"),
        (9, "IX"),
        (5, "V"),
        (4, "IV"),
        (1, "I"),
    ] {
        while rest >= number {
            rest -= number;
            out.push_str(symbol);
        }
    }
    vec![UtilityCandidate::new(out, "罗马数字")]
}

fn parse_radix_integer(arg: &str) -> Option<i64> {
    let compact = arg.trim().replace(['_', ' '], "");
    let (negative, body) = compact
        .strip_prefix('-')
        .map(|rest| (true, rest))
        .unwrap_or((false, compact.as_str()));
    let (radix, digits) = if let Some(rest) = body.strip_prefix("0x") {
        (16, rest)
    } else if let Some(rest) = body.strip_prefix("0b") {
        (2, rest)
    } else if let Some(rest) = body.strip_prefix("0o") {
        (8, rest)
    } else {
        (10, body)
    };
    let value = i64::from_str_radix(digits, radix).ok()?;
    Some(if negative { -value } else { value })
}

fn radix_candidates(arg: &str) -> Vec<UtilityCandidate> {
    let Some(value) = parse_radix_integer(&arg.to_ascii_lowercase()) else {
        return Vec::new();
    };
    let magnitude = value.unsigned_abs();
    let sign = if value < 0 { "-" } else { "" };
    vec![
        UtilityCandidate::new(format!("{sign}0x{magnitude:X}"), "十六进制"),
        UtilityCandidate::new(format!("{sign}0b{magnitude:b}"), "二进制"),
        UtilityCandidate::new(format!("{sign}0o{magnitude:o}"), "八进制"),
        UtilityCandidate::new(value.to_string(), "十进制"),
    ]
}

fn parse_plain_number(arg: &str) -> Option<f64> {
    let value = arg
        .trim()
        .replace([',', '_', ' '], "")
        .parse::<f64>()
        .ok()?;
    value.is_finite().then_some(value)
}

fn percent_candidates(arg: &str) -> Vec<UtilityCandidate> {
    let Some(value) = parse_plain_number(arg) else {
        return Vec::new();
    };
    vec![UtilityCandidate::new(
        format!("{}%", format_number(value * 100.0)),
        "百分比",
    )]
}

fn byte_candidates(arg: &str) -> Vec<UtilityCandidate> {
    let Some(value) = parse_plain_number(arg) else {
        return Vec::new();
    };
    if value < 0.0 {
        return Vec::new();
    }
    let units = ["B", "KiB", "MiB", "GiB", "TiB", "PiB"];
    let mut scaled = value;
    let mut idx = 0usize;
    while scaled >= 1024.0 && idx + 1 < units.len() {
        scaled /= 1024.0;
        idx += 1;
    }
    let mut out = vec![UtilityCandidate::new(
        format!("{} {}", format_number(scaled), units[idx]),
        "二进制容量",
    )];
    if value >= 1000.0 {
        let decimal_units = ["B", "kB", "MB", "GB", "TB", "PB"];
        let mut decimal = value;
        let mut decimal_idx = 0usize;
        while decimal >= 1000.0 && decimal_idx + 1 < decimal_units.len() {
            decimal /= 1000.0;
            decimal_idx += 1;
        }
        out.push(UtilityCandidate::new(
            format!("{} {}", format_number(decimal), decimal_units[decimal_idx]),
            "十进制容量",
        ));
    }
    out
}

fn calculator_candidates(arg: &str) -> Vec<UtilityCandidate> {
    if arg.chars().count() > MAX_INPUT_CHARS {
        return Vec::new();
    }
    if let Some(converted) = convert_units(arg) {
        return vec![UtilityCandidate::new(converted, "单位换算")];
    }
    let Ok(value) = ExpressionParser::new(arg).parse() else {
        return Vec::new();
    };
    vec![UtilityCandidate::new(format_number(value), "计算结果")]
}

fn unit_conversion_candidates(arg: &str) -> Vec<UtilityCandidate> {
    convert_units(arg)
        .map(|value| vec![UtilityCandidate::new(value, "单位换算")])
        .unwrap_or_default()
}

fn normalize_unit(unit: &str) -> &str {
    match unit {
        "米" => "m",
        "千米" | "公里" => "km",
        "厘米" => "cm",
        "毫米" => "mm",
        "英寸" => "in",
        "英尺" => "ft",
        "千克" | "公斤" => "kg",
        "克" => "g",
        "磅" => "lb",
        "摄氏度" | "℃" => "c",
        "华氏度" | "℉" => "f",
        _ => unit,
    }
}

fn convert_units(arg: &str) -> Option<String> {
    let compact = arg.trim().to_ascii_lowercase().replace(' ', "");
    let (left, target) = compact
        .split_once("to")
        .or_else(|| compact.split_once("->"))?;
    let split = left
        .char_indices()
        .find(|(_, ch)| {
            ch.is_ascii_alphabetic()
                || matches!(
                    ch,
                    '米' | '千'
                        | '公'
                        | '厘'
                        | '毫'
                        | '英'
                        | '克'
                        | '斤'
                        | '磅'
                        | '摄'
                        | '华'
                        | '℃'
                        | '℉'
                )
        })
        .map(|(idx, _)| idx)?;
    let value = parse_plain_number(&left[..split])?;
    let from = normalize_unit(&left[split..]);
    let to = normalize_unit(target);
    let result = match (from, to) {
        ("m", "km") => value / 1000.0,
        ("m", "cm") => value * 100.0,
        ("m", "mm") => value * 1000.0,
        ("km", "m") => value * 1000.0,
        ("cm", "m") => value / 100.0,
        ("mm", "m") => value / 1000.0,
        ("in", "cm") => value * 2.54,
        ("cm", "in") => value / 2.54,
        ("ft", "m") => value * 0.3048,
        ("m", "ft") => value / 0.3048,
        ("kg", "g") => value * 1000.0,
        ("g", "kg") => value / 1000.0,
        ("kg", "lb") => value * 2.204_622_621_8,
        ("lb", "kg") => value / 2.204_622_621_8,
        ("c", "f") => value * 9.0 / 5.0 + 32.0,
        ("f", "c") => (value - 32.0) * 5.0 / 9.0,
        _ if from == to => value,
        _ => return None,
    };
    Some(format!(
        "{} {}",
        format_number(result),
        target_unit_label(to)
    ))
}

fn target_unit_label(unit: &str) -> &str {
    match unit {
        "c" => "°C",
        "f" => "°F",
        _ => unit,
    }
}

fn format_number(value: f64) -> String {
    if !value.is_finite() {
        return String::new();
    }
    let normalized = if value.abs() < 0.000_000_000_001 {
        0.0
    } else {
        value
    };
    if normalized.abs() <= 9_007_199_254_740_992.0
        && (normalized - normalized.round()).abs() < 0.000_000_000_001
    {
        return format!("{:.0}", normalized);
    }
    let mut out = format!("{normalized:.12}");
    while out.ends_with('0') {
        out.pop();
    }
    if out.ends_with('.') {
        out.pop();
    }
    out
}

struct ExpressionParser<'a> {
    input: &'a [u8],
    pos: usize,
}

impl<'a> ExpressionParser<'a> {
    fn new(input: &'a str) -> Self {
        Self {
            input: input.as_bytes(),
            pos: 0,
        }
    }

    fn parse(mut self) -> Result<f64, ()> {
        let value = self.parse_sum()?;
        self.skip_spaces();
        if self.pos != self.input.len() || !value.is_finite() {
            return Err(());
        }
        Ok(value)
    }

    fn parse_sum(&mut self) -> Result<f64, ()> {
        let mut value = self.parse_product()?;
        loop {
            self.skip_spaces();
            if self.consume(b'+') {
                value += self.parse_product()?;
            } else if self.consume(b'-') {
                value -= self.parse_product()?;
            } else {
                return Ok(value);
            }
        }
    }

    fn parse_product(&mut self) -> Result<f64, ()> {
        let mut value = self.parse_power()?;
        loop {
            self.skip_spaces();
            if self.consume(b'*') {
                value *= self.parse_power()?;
            } else if self.consume(b'/') {
                let divisor = self.parse_power()?;
                if divisor == 0.0 {
                    return Err(());
                }
                value /= divisor;
            } else if self.consume(b'%') {
                let divisor = self.parse_power()?;
                if divisor == 0.0 {
                    return Err(());
                }
                value %= divisor;
            } else {
                return Ok(value);
            }
        }
    }

    fn parse_power(&mut self) -> Result<f64, ()> {
        let base = self.parse_unary()?;
        self.skip_spaces();
        if self.consume(b'^') {
            let exponent = self.parse_power()?;
            let value = base.powf(exponent);
            value.is_finite().then_some(value).ok_or(())
        } else {
            Ok(base)
        }
    }

    fn parse_unary(&mut self) -> Result<f64, ()> {
        self.skip_spaces();
        if self.consume(b'+') {
            self.parse_unary()
        } else if self.consume(b'-') {
            Ok(-self.parse_unary()?)
        } else {
            self.parse_primary()
        }
    }

    fn parse_primary(&mut self) -> Result<f64, ()> {
        self.skip_spaces();
        if self.consume(b'(') {
            let value = self.parse_sum()?;
            self.skip_spaces();
            if !self.consume(b')') {
                return Err(());
            }
            return Ok(value);
        }
        self.parse_number()
    }

    fn parse_number(&mut self) -> Result<f64, ()> {
        self.skip_spaces();
        let start = self.pos;
        let mut seen_dot = false;
        while let Some(byte) = self.input.get(self.pos).copied() {
            if byte.is_ascii_digit() {
                self.pos += 1;
            } else if byte == b'.' && !seen_dot {
                seen_dot = true;
                self.pos += 1;
            } else {
                break;
            }
        }
        if self.pos == start || &self.input[start..self.pos] == b"." {
            return Err(());
        }
        std::str::from_utf8(&self.input[start..self.pos])
            .ok()
            .and_then(|value| value.parse::<f64>().ok())
            .filter(|value| value.is_finite())
            .ok_or(())
    }

    fn skip_spaces(&mut self) {
        while self
            .input
            .get(self.pos)
            .is_some_and(|byte| byte.is_ascii_whitespace())
        {
            self.pos += 1;
        }
    }

    fn consume(&mut self, expected: u8) -> bool {
        if self.input.get(self.pos) == Some(&expected) {
            self.pos += 1;
            true
        } else {
            false
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn relative_dates_cover_offsets_weekdays_and_month_boundaries() {
        let today = NaiveDate::from_ymd_opt(2026, 7, 25).unwrap();
        assert_eq!(
            relative_date_candidates_from(today, "mingtian").unwrap()[1].phrase,
            "2026-07-26"
        );
        assert_eq!(
            relative_date_candidates_from(today, "+3").unwrap()[1].phrase,
            "2026-07-28"
        );
        assert_eq!(
            relative_date_candidates_from(today, "xiazhouyi").unwrap()[1].phrase,
            "2026-07-27"
        );
        assert_eq!(
            relative_date_candidates_from(today, "monthend").unwrap()[1].phrase,
            "2026-07-31"
        );
    }

    #[test]
    fn number_formats_are_stable() {
        assert_eq!(
            chinese_integer("12345", false).as_deref(),
            Some("一万二千三百四十五")
        );
        assert_eq!(chinese_integer("10001", false).as_deref(), Some("一万零一"));
        assert_eq!(chinese_integer("-20", true).as_deref(), Some("负贰拾"));
        assert_eq!(fullwidth_candidates("abc 123")[0].phrase, "ａｂｃ　１２３");
        assert_eq!(roman_candidates("1994")[0].phrase, "MCMXCIV");
        assert_eq!(radix_candidates("255")[0].phrase, "0xFF");
        assert_eq!(percent_candidates("0.125")[0].phrase, "12.5%");
        assert_eq!(byte_candidates("1048576")[0].phrase, "1 MiB");
    }

    #[test]
    fn calculator_obeys_precedence_and_rejects_bad_input() {
        assert_eq!(calculator_candidates("23*17")[0].phrase, "391");
        assert_eq!(calculator_candidates("(2+3)^3")[0].phrase, "125");
        assert_eq!(calculator_candidates("100/7")[0].phrase, "14.285714285714");
        assert!(calculator_candidates("1/0").is_empty());
        assert!(calculator_candidates("2+evil").is_empty());
    }

    #[test]
    fn calculator_converts_common_units() {
        assert_eq!(calculator_candidates("3.5km to m")[0].phrase, "3500 m");
        assert_eq!(calculator_candidates("30c to f")[0].phrase, "86 °F");
        assert_eq!(calculator_candidates("12in to cm")[0].phrase, "30.48 cm");
    }
}
