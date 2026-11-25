//! Keyboard layout definitions for Russian (ЙЦУКЕН), Japanese (50-on pure kana),
//! and Korean (2-Beolsik / 두벌식), with optional phonetic/romanization hints.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Language {
    Russian,
    Japanese,
    Korean,
}

impl Language {
    pub const ALL: [Self; 3] = [Self::Russian, Self::Japanese, Self::Korean];

    pub fn label(self) -> &'static str {
        match self {
            Self::Russian => "俄语 RU",
            Self::Japanese => "日语 JP",
            Self::Korean => "韩语 KO",
        }
    }

    pub fn subtitle(self) -> &'static str {
        match self {
            Self::Russian => "标准 ЙЦУКЕН",
            Self::Japanese => "五十音纯假名",
            Self::Korean => "二套式 2-Beolsik",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyTone {
    Standard,
    Consonant,
    Vowel,
    Modifier,
    Accent,
    Punctuation,
}

#[derive(Debug, Clone, Copy)]
pub struct CharKey {
    pub normal: &'static str,
    pub shifted: &'static str,
    pub hint: &'static str,
    pub shifted_hint: &'static str,
    pub tone: KeyTone,
}

impl CharKey {
    pub const fn new(
        normal: &'static str,
        shifted: &'static str,
        hint: &'static str,
        tone: KeyTone,
    ) -> Self {
        Self {
            normal,
            shifted,
            hint,
            shifted_hint: hint,
            tone,
        }
    }

    pub const fn with_shifted_hint(
        normal: &'static str,
        shifted: &'static str,
        hint: &'static str,
        shifted_hint: &'static str,
        tone: KeyTone,
    ) -> Self {
        Self {
            normal,
            shifted,
            hint,
            shifted_hint,
            tone,
        }
    }

    pub fn active_text(&self, shifted: bool) -> &'static str {
        if shifted {
            self.shifted
        } else {
            self.normal
        }
    }

    pub fn alternate_text(&self, shifted: bool) -> Option<&'static str> {
        if self.normal == self.shifted {
            None
        } else if shifted {
            Some(self.normal)
        } else {
            Some(self.shifted)
        }
    }

    pub fn active_hint(&self, shifted: bool) -> &'static str {
        if shifted {
            self.shifted_hint
        } else {
            self.hint
        }
    }
}

#[derive(Debug, Clone, Copy)]
pub struct KanaKey {
    pub hiragana: char,
    pub katakana: char,
    pub romaji: &'static str,
}

impl KanaKey {
    pub const fn new(hiragana: char, katakana: char, romaji: &'static str) -> Self {
        Self {
            hiragana,
            katakana,
            romaji,
        }
    }

    pub fn ch(&self, katakana_mode: bool) -> char {
        if katakana_mode {
            self.katakana
        } else {
            self.hiragana
        }
    }
}

// ============================================================================
// Russian (Windows Standard ЙЦУКЕН, 33 Cyrillic letters + digits & punctuation)
// ============================================================================

pub const RU_ROW_1: &[CharKey] = &[
    CharKey::new("ё", "Ё", "yo", KeyTone::Standard),
    CharKey::new("1", "!", "", KeyTone::Punctuation),
    CharKey::new("2", "\"", "", KeyTone::Punctuation),
    CharKey::new("3", "№", "", KeyTone::Punctuation),
    CharKey::new("4", ";", "", KeyTone::Punctuation),
    CharKey::new("5", "%", "", KeyTone::Punctuation),
    CharKey::new("6", ":", "", KeyTone::Punctuation),
    CharKey::new("7", "?", "", KeyTone::Punctuation),
    CharKey::new("8", "*", "", KeyTone::Punctuation),
    CharKey::new("9", "(", "", KeyTone::Punctuation),
    CharKey::new("0", ")", "", KeyTone::Punctuation),
    CharKey::new("-", "_", "", KeyTone::Punctuation),
    CharKey::new("=", "+", "", KeyTone::Punctuation),
];

pub const RU_ROW_2: &[CharKey] = &[
    CharKey::new("й", "Й", "y", KeyTone::Standard),
    CharKey::new("ц", "Ц", "ts", KeyTone::Standard),
    CharKey::new("у", "У", "u", KeyTone::Standard),
    CharKey::new("к", "К", "k", KeyTone::Standard),
    CharKey::new("е", "Е", "ye", KeyTone::Standard),
    CharKey::new("н", "Н", "n", KeyTone::Standard),
    CharKey::new("г", "Г", "g", KeyTone::Standard),
    CharKey::new("ш", "Ш", "sh", KeyTone::Standard),
    CharKey::new("щ", "Щ", "shch", KeyTone::Standard),
    CharKey::new("з", "З", "z", KeyTone::Standard),
    CharKey::new("х", "Х", "kh", KeyTone::Standard),
    CharKey::new("ъ", "Ъ", "\"", KeyTone::Standard),
];

pub const RU_ROW_3: &[CharKey] = &[
    CharKey::new("ф", "Ф", "f", KeyTone::Standard),
    CharKey::new("ы", "Ы", "y", KeyTone::Standard),
    CharKey::new("в", "В", "v", KeyTone::Standard),
    CharKey::new("а", "А", "a", KeyTone::Standard),
    CharKey::new("п", "П", "p", KeyTone::Standard),
    CharKey::new("р", "Р", "r", KeyTone::Standard),
    CharKey::new("о", "О", "o", KeyTone::Standard),
    CharKey::new("л", "Л", "l", KeyTone::Standard),
    CharKey::new("д", "Д", "d", KeyTone::Standard),
    CharKey::new("ж", "Ж", "zh", KeyTone::Standard),
    CharKey::new("э", "Э", "e", KeyTone::Standard),
];

pub const RU_ROW_4: &[CharKey] = &[
    CharKey::new("я", "Я", "ya", KeyTone::Standard),
    CharKey::new("ч", "Ч", "ch", KeyTone::Standard),
    CharKey::new("с", "С", "s", KeyTone::Standard),
    CharKey::new("м", "М", "m", KeyTone::Standard),
    CharKey::new("и", "И", "i", KeyTone::Standard),
    CharKey::new("т", "Т", "t", KeyTone::Standard),
    CharKey::new("ь", "Ь", "'", KeyTone::Standard),
    CharKey::new("б", "Б", "b", KeyTone::Standard),
    CharKey::new("ю", "Ю", "yu", KeyTone::Standard),
    CharKey::new(".", ",", "", KeyTone::Punctuation),
];

// ============================================================================
// Japanese (50-on Gojūon grid: 5 vowel rows × 10 consonant columns)
// ============================================================================

pub const JA_GOJUON_ROWS: [[Option<KanaKey>; 10]; 5] = [
    [
        Some(KanaKey::new('あ', 'ア', "a")),
        Some(KanaKey::new('か', 'カ', "ka")),
        Some(KanaKey::new('さ', 'サ', "sa")),
        Some(KanaKey::new('た', 'タ', "ta")),
        Some(KanaKey::new('な', 'ナ', "na")),
        Some(KanaKey::new('は', 'ハ', "ha")),
        Some(KanaKey::new('ま', 'マ', "ma")),
        Some(KanaKey::new('や', 'ヤ', "ya")),
        Some(KanaKey::new('ら', 'ラ', "ra")),
        Some(KanaKey::new('わ', 'ワ', "wa")),
    ],
    [
        Some(KanaKey::new('い', 'イ', "i")),
        Some(KanaKey::new('き', 'キ', "ki")),
        Some(KanaKey::new('し', 'シ', "shi")),
        Some(KanaKey::new('ち', 'チ', "chi")),
        Some(KanaKey::new('に', 'ニ', "ni")),
        Some(KanaKey::new('ひ', 'ヒ', "hi")),
        Some(KanaKey::new('み', 'ミ', "mi")),
        None,
        Some(KanaKey::new('り', 'リ', "ri")),
        Some(KanaKey::new('を', 'ヲ', "wo")),
    ],
    [
        Some(KanaKey::new('う', 'ウ', "u")),
        Some(KanaKey::new('く', 'ク', "ku")),
        Some(KanaKey::new('す', 'ス', "su")),
        Some(KanaKey::new('つ', 'ツ', "tsu")),
        Some(KanaKey::new('ぬ', 'ヌ', "nu")),
        Some(KanaKey::new('ふ', 'フ', "fu")),
        Some(KanaKey::new('む', 'ム', "mu")),
        Some(KanaKey::new('ゆ', 'ユ', "yu")),
        Some(KanaKey::new('る', 'ル', "ru")),
        Some(KanaKey::new('ん', 'ン', "n")),
    ],
    [
        Some(KanaKey::new('え', 'エ', "e")),
        Some(KanaKey::new('け', 'ケ', "ke")),
        Some(KanaKey::new('せ', 'セ', "se")),
        Some(KanaKey::new('て', 'テ', "te")),
        Some(KanaKey::new('ね', 'ネ', "ne")),
        Some(KanaKey::new('へ', 'ヘ', "he")),
        Some(KanaKey::new('め', 'メ', "me")),
        None,
        Some(KanaKey::new('れ', 'レ', "re")),
        Some(KanaKey::new('ー', 'ー', "—")),
    ],
    [
        Some(KanaKey::new('お', 'オ', "o")),
        Some(KanaKey::new('こ', 'コ', "ko")),
        Some(KanaKey::new('そ', 'ソ', "so")),
        Some(KanaKey::new('と', 'ト', "to")),
        Some(KanaKey::new('の', 'ノ', "no")),
        Some(KanaKey::new('ほ', 'ホ', "ho")),
        Some(KanaKey::new('も', 'モ', "mo")),
        Some(KanaKey::new('よ', 'ヨ', "yo")),
        Some(KanaKey::new('ろ', 'ロ', "ro")),
        Some(KanaKey::new('・', '・', "·")),
    ],
];

// ============================================================================
// Korean (Standard KS X 5002 2-Beolsik / 두벌식)
// ============================================================================

pub const KO_ROW_1: &[CharKey] = &[
    CharKey::new("1", "!", "", KeyTone::Punctuation),
    CharKey::new("2", "@", "", KeyTone::Punctuation),
    CharKey::new("3", "#", "", KeyTone::Punctuation),
    CharKey::new("4", "$", "", KeyTone::Punctuation),
    CharKey::new("5", "%", "", KeyTone::Punctuation),
    CharKey::new("6", "^", "", KeyTone::Punctuation),
    CharKey::new("7", "&", "", KeyTone::Punctuation),
    CharKey::new("8", "*", "", KeyTone::Punctuation),
    CharKey::new("9", "(", "", KeyTone::Punctuation),
    CharKey::new("0", ")", "", KeyTone::Punctuation),
    CharKey::new("-", "_", "", KeyTone::Punctuation),
    CharKey::new("=", "+", "", KeyTone::Punctuation),
];

pub const KO_ROW_2: &[CharKey] = &[
    CharKey::with_shifted_hint("ㅂ", "ㅃ", "b", "pp", KeyTone::Consonant),
    CharKey::with_shifted_hint("ㅈ", "ㅉ", "j", "jj", KeyTone::Consonant),
    CharKey::with_shifted_hint("ㄷ", "ㄸ", "d", "tt", KeyTone::Consonant),
    CharKey::with_shifted_hint("ㄱ", "ㄲ", "g", "kk", KeyTone::Consonant),
    CharKey::with_shifted_hint("ㅅ", "ㅆ", "s", "ss", KeyTone::Consonant),
    CharKey::new("ㅛ", "ㅛ", "yo", KeyTone::Vowel),
    CharKey::new("ㅕ", "ㅕ", "yeo", KeyTone::Vowel),
    CharKey::new("ㅑ", "ㅑ", "ya", KeyTone::Vowel),
    CharKey::with_shifted_hint("ㅐ", "ㅒ", "ae", "yae", KeyTone::Vowel),
    CharKey::with_shifted_hint("ㅔ", "ㅖ", "e", "ye", KeyTone::Vowel),
];

pub const KO_ROW_3: &[CharKey] = &[
    CharKey::new("ㅁ", "ㅁ", "m", KeyTone::Consonant),
    CharKey::new("ㄴ", "ㄴ", "n", KeyTone::Consonant),
    CharKey::new("ㅇ", "ㅇ", "ng", KeyTone::Consonant),
    CharKey::new("ㄹ", "ㄹ", "r/l", KeyTone::Consonant),
    CharKey::new("ㅎ", "ㅎ", "h", KeyTone::Consonant),
    CharKey::new("ㅗ", "ㅗ", "o", KeyTone::Vowel),
    CharKey::new("ㅓ", "ㅓ", "eo", KeyTone::Vowel),
    CharKey::new("ㅏ", "ㅏ", "a", KeyTone::Vowel),
    CharKey::new("ㅣ", "ㅣ", "i", KeyTone::Vowel),
];

pub const KO_ROW_4: &[CharKey] = &[
    CharKey::new("ㅋ", "ㅋ", "k", KeyTone::Consonant),
    CharKey::new("ㅌ", "ㅌ", "t", KeyTone::Consonant),
    CharKey::new("ㅊ", "ㅊ", "ch", KeyTone::Consonant),
    CharKey::new("ㅍ", "ㅍ", "p", KeyTone::Consonant),
    CharKey::new("ㅠ", "ㅠ", "yu", KeyTone::Vowel),
    CharKey::new("ㅜ", "ㅜ", "u", KeyTone::Vowel),
    CharKey::new("ㅡ", "ㅡ", "eu", KeyTone::Vowel),
    CharKey::new(",", "?", "", KeyTone::Punctuation),
    CharKey::new(".", "!", "", KeyTone::Punctuation),
];

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashSet;

    #[test]
    fn russian_layout_covers_all_33_cyrillic_letters() {
        let letters: HashSet<char> = RU_ROW_1
            .iter()
            .chain(RU_ROW_2)
            .chain(RU_ROW_3)
            .chain(RU_ROW_4)
            .filter_map(|k| k.normal.chars().next())
            .filter(|c| ('а'..='я').contains(c) || *c == 'ё')
            .collect();
        assert_eq!(letters.len(), 33);
        assert!(letters.contains(&'ё'));
    }

    #[test]
    fn japanese_gojuon_covers_46_basic_kana_in_both_scripts() {
        let mut hira = HashSet::new();
        let mut kata = HashSet::new();
        for row in JA_GOJUON_ROWS {
            for cell in row.into_iter().flatten() {
                if cell.hiragana != 'ー' && cell.hiragana != '・' {
                    hira.insert(cell.hiragana);
                    kata.insert(cell.katakana);
                }
            }
        }
        assert_eq!(hira.len(), 46);
        assert_eq!(kata.len(), 46);
    }

    #[test]
    fn korean_2_beolsik_covers_all_19_consonants_and_14_base_vowels() {
        let mut jamo = HashSet::new();
        for key in KO_ROW_2.iter().chain(KO_ROW_3).chain(KO_ROW_4) {
            if let Some(ch) = key.normal.chars().next() {
                if ('ㄱ'..='ㅣ').contains(&ch) {
                    jamo.insert(ch);
                }
            }
            if let Some(ch) = key.shifted.chars().next() {
                if ('ㄱ'..='ㅣ').contains(&ch) {
                    jamo.insert(ch);
                }
            }
        }
        assert_eq!(jamo.len(), 33); // 19 consonants + 14 vowels
    }
}
