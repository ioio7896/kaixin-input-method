# Candidate Quality Evaluation

`tests/candidate_quality_cases.sqlite` is a fixed candidate-ranking benchmark for daily tuning.
It covers common words, sentences, jianpin, mixed pinyin, typo correction, special pinyin,
Hangzhou/Zhejiang terms, medical terms, and poetry/place-name entries.

`tests/short_abbrev_cases.sqlite` focuses on short abbreviation quality under the current
`core/base/ext/large/en` lexicon layout. It tracks ext-layer short inputs, common/base words
that should remain visible within a small TOPN window, and short mixed-prefix inputs such as
`beij` -> 北京.

`tests/mixed_pinyin_cases.sqlite` focuses on mixed pinyin prefix input where at least one syllable
prefix is explicit, such as `shur` -> 输入 and `shuruf` -> 输入法. Pure 2-3 letter abbreviations
belong in `short_abbrev_cases.sqlite`.

Run it from `pinyin-ime`:

```powershell
cargo run --bin input_eval -- --quality
```

For the short abbreviation benchmark:

```powershell
cargo run --bin input_eval -- --cases ..\tests\short_abbrev_cases.sqlite
```

For the mixed-prefix benchmark:

```powershell
cargo run --bin input_eval -- --cases ..\tests\mixed_pinyin_cases.sqlite
```

Use these focused benchmarks as trend metrics first; add `--min-top1`, `--min-top3`, or
`--min-top9` only after agreeing on stable baselines for each case set.

The report prints per-case previews, total top1/top3/top9 hit rates, and per-category hit rates.
Use gates when running in CI:

```powershell
cargo run --bin input_eval -- --quality --min-top1 85 --min-top3 85 --min-top9 85
```

To inspect a single case:

```powershell
cargo run --bin input_eval -- --quality --explain typo_jntian --explain-limit 30
```

To inspect every case with source-layer and match-kind annotations:

```powershell
cargo run --bin input_eval -- --cases ..\tests\mixed_pinyin_cases.sqlite --explain-all
```

To collect lookup latency numbers for the same fixed workload:

```powershell
cargo run --bin input_perf -- --workload-size 300 --warmup 2 --iterations 3
```

The report prints average, p50, p95, p99, and max lookup latency. Optional gates
can be added with `--max-p95-us` and `--max-p99-us`.

For the independent 1,000-item popular four-character benchmark and its full,
abbreviation, mixed, alternating, and syllable-prefix variants:

```powershell
python scripts\build_popular_four_char_cases.py --limit 1000
cd pinyin-ime
cargo run --bin phrase_len_eval -- --only-popular
```

Use `--popular-limit 100` for a faster tuning loop. The benchmark membership and
frequency come from `pinyin-ime/data/corpus.txt`, rather than from candidate
lexicon frequency, so it can expose coverage gaps.

Case table schema:

```sql
CREATE TABLE cases (
  sort_order INTEGER NOT NULL,
  case_id TEXT PRIMARY KEY,
  category TEXT NOT NULL,
  input TEXT NOT NULL,
  expected TEXT NOT NULL
);
```

`expected` supports exact matches plus `@TOPN:<text>`, `@DATE`, `@TIME`, and `@REGEX:<regex>`.
