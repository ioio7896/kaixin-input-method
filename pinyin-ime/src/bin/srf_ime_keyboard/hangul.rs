//! Pure-Rust Korean 2-Beolsik (두벌식) Hangul syllable composition state machine.
//! Supports compound vowels (ㅘ..ㅢ), compound finals (ㄳ..ㅄ), automatic
//! syllable splitting when a vowel follows a final consonant (도깨비불),
//! and jamo-level backspace.

const HANGUL_BASE: u32 = 0xAC00;

const CHOSEONG: [char; 19] = [
    'ㄱ', 'ㄲ', 'ㄴ', 'ㄷ', 'ㄸ', 'ㄹ', 'ㅁ', 'ㅂ', 'ㅃ', 'ㅅ', 'ㅆ', 'ㅇ', 'ㅈ', 'ㅉ', 'ㅊ', 'ㅋ',
    'ㅌ', 'ㅍ', 'ㅎ',
];

const JUNGSEONG: [char; 21] = [
    'ㅏ', 'ㅐ', 'ㅑ', 'ㅒ', 'ㅓ', 'ㅔ', 'ㅕ', 'ㅖ', 'ㅗ', 'ㅘ', 'ㅙ', 'ㅚ', 'ㅛ', 'ㅜ', 'ㅝ', 'ㅞ',
    'ㅟ', 'ㅠ', 'ㅡ', 'ㅢ', 'ㅣ',
];

const JONGSEONG: [Option<char>; 28] = [
    None,
    Some('ㄱ'),
    Some('ㄲ'),
    Some('ㄳ'),
    Some('ㄴ'),
    Some('ㄵ'),
    Some('ㄶ'),
    Some('ㄷ'),
    Some('ㄹ'),
    Some('ㄺ'),
    Some('ㄻ'),
    Some('ㄼ'),
    Some('ㄽ'),
    Some('ㄾ'),
    Some('ㄿ'),
    Some('ㅀ'),
    Some('ㅁ'),
    Some('ㅂ'),
    Some('ㅄ'),
    Some('ㅅ'),
    Some('ㅆ'),
    Some('ㅇ'),
    Some('ㅈ'),
    Some('ㅊ'),
    Some('ㅋ'),
    Some('ㅌ'),
    Some('ㅍ'),
    Some('ㅎ'),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum State {
    #[default]
    Empty,
    StandaloneVowel(char),
    Lead(char),
    LeadVowel {
        lead: char,
        vowel: char,
    },
    LeadVowelTail {
        lead: char,
        vowel: char,
        tail: char,
    },
}

#[derive(Debug, Default)]
pub struct HangulComposer {
    state: State,
}

impl HangulComposer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.state == State::Empty
    }

    pub fn preview(&self) -> Option<char> {
        match self.state {
            State::Empty => None,
            State::StandaloneVowel(v) => Some(v),
            State::Lead(l) => Some(l),
            State::LeadVowel { lead, vowel } => compose_syllable(lead, vowel, None),
            State::LeadVowelTail { lead, vowel, tail } => compose_syllable(lead, vowel, Some(tail)),
        }
    }

    pub fn flush(&mut self) -> Option<String> {
        let out = self.preview().map(|c| c.to_string());
        self.state = State::Empty;
        out
    }

    pub fn clear(&mut self) {
        self.state = State::Empty;
    }

    /// Push a single Hangul jamo (consonant or vowel).
    /// Returns any finalized text that should be committed immediately.
    pub fn push_jamo(&mut self, ch: char) -> Option<String> {
        if is_consonant(ch) {
            self.push_consonant(ch)
        } else if is_vowel(ch) {
            self.push_vowel(ch)
        } else {
            let mut committed = self.flush().unwrap_or_default();
            committed.push(ch);
            Some(committed)
        }
    }

    /// Step back one jamo inside the active syllable.
    /// Returns `true` if consumed internally, `false` if already empty.
    pub fn backspace(&mut self) -> bool {
        match self.state {
            State::Empty => false,
            State::StandaloneVowel(v) => {
                self.state = match split_vowel(v) {
                    Some(base) => State::StandaloneVowel(base),
                    None => State::Empty,
                };
                true
            }
            State::Lead(_) => {
                self.state = State::Empty;
                true
            }
            State::LeadVowel { lead, vowel } => {
                self.state = match split_vowel(vowel) {
                    Some(base) => State::LeadVowel { lead, vowel: base },
                    None => State::Lead(lead),
                };
                true
            }
            State::LeadVowelTail { lead, vowel, tail } => {
                self.state = match split_final(tail) {
                    Some((first, _)) => State::LeadVowelTail {
                        lead,
                        vowel,
                        tail: first,
                    },
                    None => State::LeadVowel { lead, vowel },
                };
                true
            }
        }
    }

    fn push_consonant(&mut self, ch: char) -> Option<String> {
        match self.state {
            State::Empty => {
                self.state = State::Lead(ch);
                None
            }
            State::StandaloneVowel(v) => {
                self.state = State::Lead(ch);
                Some(v.to_string())
            }
            State::Lead(l) => {
                self.state = State::Lead(ch);
                Some(l.to_string())
            }
            State::LeadVowel { lead, vowel } => {
                if jongseong_index(ch).is_some() {
                    self.state = State::LeadVowelTail {
                        lead,
                        vowel,
                        tail: ch,
                    };
                    None
                } else {
                    let committed = compose_syllable(lead, vowel, None).map(|c| c.to_string());
                    self.state = State::Lead(ch);
                    committed
                }
            }
            State::LeadVowelTail { lead, vowel, tail } => {
                if let Some(combined) = combine_final(tail, ch) {
                    self.state = State::LeadVowelTail {
                        lead,
                        vowel,
                        tail: combined,
                    };
                    None
                } else {
                    let committed =
                        compose_syllable(lead, vowel, Some(tail)).map(|c| c.to_string());
                    self.state = State::Lead(ch);
                    committed
                }
            }
        }
    }

    fn push_vowel(&mut self, ch: char) -> Option<String> {
        match self.state {
            State::Empty => {
                self.state = State::StandaloneVowel(ch);
                None
            }
            State::StandaloneVowel(v) => {
                if let Some(combined) = combine_vowel(v, ch) {
                    self.state = State::StandaloneVowel(combined);
                    None
                } else {
                    self.state = State::StandaloneVowel(ch);
                    Some(v.to_string())
                }
            }
            State::Lead(lead) => {
                self.state = State::LeadVowel { lead, vowel: ch };
                None
            }
            State::LeadVowel { lead, vowel } => {
                if let Some(combined) = combine_vowel(vowel, ch) {
                    self.state = State::LeadVowel {
                        lead,
                        vowel: combined,
                    };
                    None
                } else {
                    let committed = compose_syllable(lead, vowel, None).map(|c| c.to_string());
                    self.state = State::StandaloneVowel(ch);
                    committed
                }
            }
            State::LeadVowelTail { lead, vowel, tail } => {
                if let Some((first, second)) = split_final(tail) {
                    let committed =
                        compose_syllable(lead, vowel, Some(first)).map(|c| c.to_string());
                    self.state = State::LeadVowel {
                        lead: second,
                        vowel: ch,
                    };
                    committed
                } else {
                    let committed = compose_syllable(lead, vowel, None).map(|c| c.to_string());
                    self.state = State::LeadVowel {
                        lead: tail,
                        vowel: ch,
                    };
                    committed
                }
            }
        }
    }
}

fn is_consonant(ch: char) -> bool {
    CHOSEONG.contains(&ch)
}

fn is_vowel(ch: char) -> bool {
    JUNGSEONG.contains(&ch)
}

fn choseong_index(ch: char) -> Option<u32> {
    CHOSEONG.iter().position(|&c| c == ch).map(|i| i as u32)
}

fn jungseong_index(ch: char) -> Option<u32> {
    JUNGSEONG.iter().position(|&c| c == ch).map(|i| i as u32)
}

fn jongseong_index(ch: char) -> Option<u32> {
    JONGSEONG
        .iter()
        .position(|&c| c == Some(ch))
        .map(|i| i as u32)
}

fn compose_syllable(lead: char, vowel: char, tail: Option<char>) -> Option<char> {
    let l = choseong_index(lead)?;
    let v = jungseong_index(vowel)?;
    let t = match tail {
        Some(ch) => jongseong_index(ch)?,
        None => 0,
    };
    char::from_u32(HANGUL_BASE + (l * 21 + v) * 28 + t)
}

fn combine_vowel(first: char, second: char) -> Option<char> {
    match (first, second) {
        ('ㅗ', 'ㅏ') => Some('ㅘ'),
        ('ㅗ', 'ㅐ') => Some('ㅙ'),
        ('ㅗ', 'ㅣ') => Some('ㅚ'),
        ('ㅜ', 'ㅓ') => Some('ㅝ'),
        ('ㅜ', 'ㅔ') => Some('ㅞ'),
        ('ㅜ', 'ㅣ') => Some('ㅟ'),
        ('ㅡ', 'ㅣ') => Some('ㅢ'),
        _ => None,
    }
}

fn split_vowel(vowel: char) -> Option<char> {
    match vowel {
        'ㅘ' | 'ㅙ' | 'ㅚ' => Some('ㅗ'),
        'ㅝ' | 'ㅞ' | 'ㅟ' => Some('ㅜ'),
        'ㅢ' => Some('ㅡ'),
        _ => None,
    }
}

fn combine_final(first: char, second: char) -> Option<char> {
    match (first, second) {
        ('ㄱ', 'ㅅ') => Some('ㄳ'),
        ('ㄴ', 'ㅈ') => Some('ㄵ'),
        ('ㄴ', 'ㅎ') => Some('ㄶ'),
        ('ㄹ', 'ㄱ') => Some('ㄺ'),
        ('ㄹ', 'ㅁ') => Some('ㄻ'),
        ('ㄹ', 'ㅂ') => Some('ㄼ'),
        ('ㄹ', 'ㅅ') => Some('ㄽ'),
        ('ㄹ', 'ㅌ') => Some('ㄾ'),
        ('ㄹ', 'ㅍ') => Some('ㄿ'),
        ('ㄹ', 'ㅎ') => Some('ㅀ'),
        ('ㅂ', 'ㅅ') => Some('ㅄ'),
        _ => None,
    }
}

fn split_final(tail: char) -> Option<(char, char)> {
    match tail {
        'ㄳ' => Some(('ㄱ', 'ㅅ')),
        'ㄵ' => Some(('ㄴ', 'ㅈ')),
        'ㄶ' => Some(('ㄴ', 'ㅎ')),
        'ㄺ' => Some(('ㄹ', 'ㄱ')),
        'ㄻ' => Some(('ㄹ', 'ㅁ')),
        'ㄼ' => Some(('ㄹ', 'ㅂ')),
        'ㄽ' => Some(('ㄹ', 'ㅅ')),
        'ㄾ' => Some(('ㄹ', 'ㅌ')),
        'ㄿ' => Some(('ㄹ', 'ㅍ')),
        'ㅀ' => Some(('ㄹ', 'ㅎ')),
        'ㅄ' => Some(('ㅂ', 'ㅅ')),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn type_sequence(jamos: &[char]) -> String {
        let mut composer = HangulComposer::new();
        let mut out = String::new();
        for &jamo in jamos {
            if let Some(committed) = composer.push_jamo(jamo) {
                out.push_str(&committed);
            }
        }
        if let Some(rest) = composer.flush() {
            out.push_str(&rest);
        }
        out
    }

    #[test]
    fn composes_basic_and_compound_syllables() {
        assert_eq!(type_sequence(&['ㅎ', 'ㅏ', 'ㄴ', 'ㄱ', 'ㅡ', 'ㄹ']), "한글");
        assert_eq!(type_sequence(&['ㄱ', 'ㅗ', 'ㅏ', 'ㅇ']), "광");
        assert_eq!(type_sequence(&['ㄷ', 'ㅏ', 'ㄹ', 'ㄱ']), "닭");
    }

    #[test]
    fn splits_single_and_compound_finals_when_vowel_follows() {
        assert_eq!(type_sequence(&['ㅎ', 'ㅏ', 'ㄴ', 'ㅏ']), "하나");
        assert_eq!(type_sequence(&['ㄷ', 'ㅏ', 'ㄹ', 'ㄱ', 'ㅏ']), "달가");
        assert_eq!(
            type_sequence(&[
                'ㅇ', 'ㅏ', 'ㄴ', 'ㄴ', 'ㅕ', 'ㅇ', 'ㅎ', 'ㅏ', 'ㅅ', 'ㅔ', 'ㅇ', 'ㅛ'
            ]),
            "안녕하세요"
        );
    }

    #[test]
    fn handles_non_final_double_consonants_and_jamo_backspace() {
        assert_eq!(type_sequence(&['ㅏ', 'ㅃ', 'ㅏ']), "ㅏ빠");

        let mut composer = HangulComposer::new();
        for ch in ['ㄷ', 'ㅗ', 'ㅏ', 'ㄹ', 'ㄱ'] {
            assert!(composer.push_jamo(ch).is_none());
        }
        assert_eq!(composer.preview(), Some('돩'));
        assert!(composer.backspace());
        assert_eq!(composer.preview(), Some('돨'));
        assert!(composer.backspace());
        assert_eq!(composer.preview(), Some('돠'));
        assert!(composer.backspace());
        assert_eq!(composer.preview(), Some('도'));
        assert!(composer.backspace());
        assert_eq!(composer.preview(), Some('ㄷ'));
        assert!(composer.backspace());
        assert_eq!(composer.preview(), None);
        assert!(!composer.backspace());
    }
}
