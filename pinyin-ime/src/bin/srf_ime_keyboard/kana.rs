//! Pure-kana Japanese composer that holds a single transformable kana so
//! dakuten (゛), handakuten (゜), small kana (小), and cycle (゛゜小)
//! work instantaneously without sending Backspace to the target editor.

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KanaTransform {
    Dakuten,
    Handakuten,
    Small,
    Cycle,
}

#[derive(Debug, Default)]
pub struct KanaComposer {
    pending: Option<char>,
}

impl KanaComposer {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn is_empty(&self) -> bool {
        self.pending.is_none()
    }

    pub fn preview(&self) -> Option<char> {
        self.pending
    }

    pub fn flush(&mut self) -> Option<String> {
        self.pending.take().map(|c| c.to_string())
    }

    pub fn clear(&mut self) {
        self.pending = None;
    }

    /// Push a kana character. Transformable kana stay in the 1-char preview
    /// slot until the next key; non-transformable kana commit immediately.
    pub fn push_kana(&mut self, ch: char) -> Option<String> {
        let prev = self.pending.take();
        if has_any_transform(ch) {
            self.pending = Some(ch);
            prev.map(|c| c.to_string())
        } else {
            let mut out = String::new();
            if let Some(p) = prev {
                out.push(p);
            }
            out.push(ch);
            Some(out)
        }
    }

    pub fn can_apply(&self, transform: KanaTransform) -> bool {
        let Some(ch) = self.pending else {
            return false;
        };
        apply_transform(ch, transform).is_some()
    }

    pub fn apply(&mut self, transform: KanaTransform) -> bool {
        let Some(ch) = self.pending else {
            return false;
        };
        if let Some(next) = apply_transform(ch, transform) {
            self.pending = Some(next);
            true
        } else {
            false
        }
    }

    pub fn backspace(&mut self) -> bool {
        self.pending.take().is_some()
    }
}

pub fn has_any_transform(ch: char) -> bool {
    toggle_dakuten(ch).is_some() || toggle_handakuten(ch).is_some() || toggle_small(ch).is_some()
}

fn apply_transform(ch: char, transform: KanaTransform) -> Option<char> {
    match transform {
        KanaTransform::Dakuten => toggle_dakuten(ch),
        KanaTransform::Handakuten => toggle_handakuten(ch),
        KanaTransform::Small => toggle_small(ch),
        KanaTransform::Cycle => cycle_kana(ch),
    }
}

fn toggle_dakuten(ch: char) -> Option<char> {
    match ch {
        // Hiragana
        'か' => Some('が'),
        'が' => Some('か'),
        'き' => Some('ぎ'),
        'ぎ' => Some('き'),
        'く' => Some('ぐ'),
        'ぐ' => Some('く'),
        'け' => Some('げ'),
        'げ' => Some('け'),
        'こ' => Some('ご'),
        'ご' => Some('こ'),
        'さ' => Some('ざ'),
        'ざ' => Some('さ'),
        'し' => Some('じ'),
        'じ' => Some('し'),
        'す' => Some('ず'),
        'ず' => Some('す'),
        'せ' => Some('ぜ'),
        'ぜ' => Some('せ'),
        'そ' => Some('ぞ'),
        'ぞ' => Some('そ'),
        'た' => Some('だ'),
        'だ' => Some('た'),
        'ち' => Some('ぢ'),
        'ぢ' => Some('ち'),
        'つ' | 'っ' => Some('づ'),
        'づ' => Some('つ'),
        'て' => Some('で'),
        'で' => Some('て'),
        'と' => Some('ど'),
        'ど' => Some('と'),
        'は' | 'ぱ' => Some('ば'),
        'ば' => Some('は'),
        'ひ' | 'ぴ' => Some('び'),
        'び' => Some('ひ'),
        'ふ' | 'ぷ' => Some('ぶ'),
        'ぶ' => Some('ふ'),
        'へ' | 'ぺ' => Some('べ'),
        'べ' => Some('へ'),
        'ほ' | 'ぽ' => Some('ぼ'),
        'ぼ' => Some('ほ'),
        'う' | 'ぅ' => Some('ゔ'),
        'ゔ' => Some('う'),
        // Katakana
        'カ' | 'ヵ' => Some('ガ'),
        'ガ' => Some('カ'),
        'キ' => Some('ギ'),
        'ギ' => Some('キ'),
        'ク' => Some('グ'),
        'グ' => Some('ク'),
        'ケ' | 'ヶ' => Some('ゲ'),
        'ゲ' => Some('ケ'),
        'コ' => Some('ゴ'),
        'ゴ' => Some('コ'),
        'サ' => Some('ザ'),
        'ザ' => Some('サ'),
        'シ' => Some('ジ'),
        'ジ' => Some('シ'),
        'ス' => Some('ズ'),
        'ズ' => Some('ス'),
        'セ' => Some('ゼ'),
        'ゼ' => Some('セ'),
        'ソ' => Some('ゾ'),
        'ゾ' => Some('ソ'),
        'タ' => Some('ダ'),
        'ダ' => Some('タ'),
        'チ' => Some('ヂ'),
        'ヂ' => Some('チ'),
        'ツ' | 'ッ' => Some('ヅ'),
        'ヅ' => Some('ツ'),
        'テ' => Some('デ'),
        'デ' => Some('テ'),
        'ト' => Some('ド'),
        'ド' => Some('ト'),
        'ハ' | 'パ' => Some('バ'),
        'バ' => Some('ハ'),
        'ヒ' | 'ピ' => Some('ビ'),
        'ビ' => Some('ヒ'),
        'フ' | 'プ' => Some('ブ'),
        'ブ' => Some('フ'),
        'ヘ' | 'ペ' => Some('ベ'),
        'ベ' => Some('ヘ'),
        'ホ' | 'ポ' => Some('ボ'),
        'ボ' => Some('ホ'),
        'ウ' | 'ゥ' => Some('ヴ'),
        'ヴ' => Some('ウ'),
        _ => None,
    }
}

fn toggle_handakuten(ch: char) -> Option<char> {
    match ch {
        'は' | 'ば' => Some('ぱ'),
        'ぱ' => Some('は'),
        'ひ' | 'び' => Some('ぴ'),
        'ぴ' => Some('ひ'),
        'ふ' | 'ぶ' => Some('ぷ'),
        'ぷ' => Some('ふ'),
        'へ' | 'べ' => Some('ぺ'),
        'ぺ' => Some('へ'),
        'ほ' | 'ぼ' => Some('ぽ'),
        'ぽ' => Some('ほ'),
        'ハ' | 'バ' => Some('パ'),
        'パ' => Some('ハ'),
        'ヒ' | 'ビ' => Some('ピ'),
        'ピ' => Some('ヒ'),
        'フ' | 'ブ' => Some('プ'),
        'プ' => Some('フ'),
        'ヘ' | 'ベ' => Some('ペ'),
        'ペ' => Some('ヘ'),
        'ホ' | 'ボ' => Some('ポ'),
        'ポ' => Some('ホ'),
        _ => None,
    }
}

fn toggle_small(ch: char) -> Option<char> {
    match ch {
        // Hiragana
        'あ' => Some('ぁ'),
        'ぁ' => Some('あ'),
        'い' => Some('ぃ'),
        'ぃ' => Some('い'),
        'う' | 'ゔ' => Some('ぅ'),
        'ぅ' => Some('う'),
        'え' => Some('ぇ'),
        'ぇ' => Some('え'),
        'お' => Some('ぉ'),
        'ぉ' => Some('お'),
        'つ' | 'づ' => Some('っ'),
        'っ' => Some('つ'),
        'や' => Some('ゃ'),
        'ゃ' => Some('や'),
        'ゆ' => Some('ゅ'),
        'ゅ' => Some('ゆ'),
        'よ' => Some('ょ'),
        'ょ' => Some('よ'),
        'わ' => Some('ゎ'),
        'ゎ' => Some('わ'),
        // Katakana
        'ア' => Some('ァ'),
        'ァ' => Some('ア'),
        'イ' => Some('ィ'),
        'ィ' => Some('イ'),
        'ウ' | 'ヴ' => Some('ゥ'),
        'ゥ' => Some('ウ'),
        'エ' => Some('ェ'),
        'ェ' => Some('エ'),
        'オ' => Some('ォ'),
        'ォ' => Some('オ'),
        'ツ' | 'ヅ' => Some('ッ'),
        'ッ' => Some('ツ'),
        'ヤ' => Some('ャ'),
        'ャ' => Some('ヤ'),
        'ユ' => Some('ュ'),
        'ュ' => Some('ユ'),
        'ヨ' => Some('ョ'),
        'ョ' => Some('ヨ'),
        'ワ' => Some('ヮ'),
        'ヮ' => Some('ワ'),
        'カ' | 'ガ' => Some('ヵ'),
        'ヵ' => Some('カ'),
        'ケ' | 'ゲ' => Some('ヶ'),
        'ヶ' => Some('ケ'),
        _ => None,
    }
}

fn cycle_kana(ch: char) -> Option<char> {
    match ch {
        // Multi-stage cycles first
        'は' => Some('ば'),
        'ば' => Some('ぱ'),
        'ぱ' => Some('は'),
        'ひ' => Some('び'),
        'び' => Some('ぴ'),
        'ぴ' => Some('ひ'),
        'ふ' => Some('ぶ'),
        'ぶ' => Some('ぷ'),
        'ぷ' => Some('ふ'),
        'へ' => Some('べ'),
        'べ' => Some('ぺ'),
        'ぺ' => Some('へ'),
        'ほ' => Some('ぼ'),
        'ぼ' => Some('ぽ'),
        'ぽ' => Some('ほ'),
        'ハ' => Some('バ'),
        'バ' => Some('パ'),
        'パ' => Some('ハ'),
        'ヒ' => Some('ビ'),
        'ビ' => Some('ピ'),
        'ピ' => Some('ヒ'),
        'フ' => Some('ブ'),
        'ブ' => Some('プ'),
        'プ' => Some('フ'),
        'ヘ' => Some('ベ'),
        'ベ' => Some('ペ'),
        'ペ' => Some('ヘ'),
        'ホ' => Some('ボ'),
        'ボ' => Some('ポ'),
        'ポ' => Some('ホ'),
        'つ' => Some('っ'),
        'っ' => Some('づ'),
        'づ' => Some('つ'),
        'ツ' => Some('ッ'),
        'ッ' => Some('ヅ'),
        'ヅ' => Some('ツ'),
        'う' => Some('ぅ'),
        'ぅ' => Some('ゔ'),
        'ゔ' => Some('う'),
        'ウ' => Some('ゥ'),
        'ゥ' => Some('ヴ'),
        'ヴ' => Some('ウ'),
        'カ' => Some('ガ'),
        'ガ' => Some('ヵ'),
        'ヵ' => Some('カ'),
        'ケ' => Some('ゲ'),
        'ゲ' => Some('ヶ'),
        'ヶ' => Some('ケ'),
        _ => toggle_dakuten(ch).or_else(|| toggle_small(ch)),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cycles_dakuten_handakuten_and_small_kana() {
        let mut composer = KanaComposer::new();
        assert_eq!(composer.push_kana('は'), None);
        assert_eq!(composer.preview(), Some('は'));
        assert!(composer.apply(KanaTransform::Cycle));
        assert_eq!(composer.preview(), Some('ば'));
        assert!(composer.apply(KanaTransform::Cycle));
        assert_eq!(composer.preview(), Some('ぱ'));
        assert!(composer.apply(KanaTransform::Cycle));
        assert_eq!(composer.preview(), Some('は'));

        assert_eq!(composer.push_kana('つ'), Some("は".into()));
        assert!(composer.apply(KanaTransform::Small));
        assert_eq!(composer.preview(), Some('っ'));
        assert_eq!(composer.push_kana('ん'), Some("っん".into()));
        assert!(composer.is_empty());
    }
}
