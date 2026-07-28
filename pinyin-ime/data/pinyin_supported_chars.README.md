# 拼音支持字符表

`pinyin_supported_chars.txt` 由 `build_clean_lexicons` 根据 `pinyin` crate 0.10.0 的可用读音生成。

生成顺序先收录许可明确词库中出现的常用字符，再补齐该 crate 支持的 CJK 字符。具体来源、固定版本和许可证见仓库根目录的 `data_sources/README.md`。
