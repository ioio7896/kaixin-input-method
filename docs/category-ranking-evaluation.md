# 分类词库排序评测

`tests/category_ranking_cases.tsv` 现在包含 40 个样例，日常、办公、专业、混拼和纠错各 8 个。它是小型人工回归集，不代表用户总体使用频率；专业词库默认关闭，专业样例只验证默认配置下的实际候选，不代表启用专业分类后的效果。

2026-11-11 词库审核将 `popular_three_char_cases.tsv` 中 7 个与拒收规则冲突的片段换为
`reviewed_complete_expression` 完整表达；这些替换行的 `rank_weight=1` 仅为占位，
不能作为词频来源。生成器从原独立语料窗口中排除已拒收片段后再抽样，生成校验会阻止
正向样例重新包含拒收词。该轮用户要求跳过测试，没有新的候选评测成绩。

以下 15 条结果是扩充前的历史记录，不能作为当前 40 条评测的成绩。当前数据由发布构建自动生成，见 `dist/input-benchmark-<版本>-<构建时间>/report.md` 和 `report.json`。报告包括 5 次新进程冷启动、首批候选管道返回、预热/增量/管道延迟的 P95/P99、工作集及峰值工作集，以及各分类 Top1/Top3。最慢输入和未命中样例保留在同目录日志及 `quality.json`。

可独立运行 `python scripts/input_benchmark_report.py --lexicon dist/kaixin-package-ime/lexicon --output dist/input-benchmark-manual`。使用安装包暂存词库和隔离用户目录，结果记录词库及样例哈希。冷启动保留操作系统磁盘缓存，首批候选不包含候选窗绘制；比较版本时需保持机器、配置和样例一致。

历史评测使用 `scripts/build_input_cases_sqlite.py --source tests/category_ranking_cases.tsv --output dist/category-ranking-cases.sqlite` 生成评测集，然后用隔离的 LOCALAPPDATA 运行 `input_eval --cases dist/category-ranking-cases.sqlite --engine-profile full --explain-all`。`full` 为文本全量索引，不含安装包的热/冷组合，另对本次构建的 IME 安装包暂存词库使用 runtime-hot-cold 配置复测，冷词库可用，15 个样例均命中 Top1。

隔离评测中，原始分数 15 个样例 Top1 为 14/15、Top3 为 15/15：`xiaomao` 的“小毛”在“小猫”之前。将基础词库“小猫”从 5072 调至 5400 的试验中，15 个样例 Top1 均命中；该分数是针对这一同音竞争的排序调整，不是实测词频。生成脚本保留该覆盖值。其他分类未因小样本整体调权。

另在隔离配置中开启人工智能、医疗、法律、财务会计分类及医疗、法律专业基础词库复测，15 个样例仍均为 Top1；该结果仅覆盖这 15 个样例。
