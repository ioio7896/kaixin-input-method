# 分类词库排序评测

`tests/category_ranking_cases.tsv` 包含 15 个完整拼音样例，各 5 个日常、办公和专业场景。它是小型人工回归集，不代表用户总体使用频率；专业词库默认关闭，专业样例只验证默认配置下的实际候选，不代表启用专业分类后的效果。

使用 `scripts/build_input_cases_sqlite.py --source tests/category_ranking_cases.tsv --output dist/category-ranking-cases.sqlite` 生成评测集，然后用隔离的 LOCALAPPDATA 运行 `input_eval --cases dist/category-ranking-cases.sqlite --engine-profile full --explain-all`。`full` 为文本全量索引，不含安装包的热/冷组合，另对本次构建的 IME 安装包暂存词库使用 runtime-hot-cold 配置复测，冷词库可用，15 个样例均命中 Top1。

隔离评测中，原始分数 15 个样例 Top1 为 14/15、Top3 为 15/15：`xiaomao` 的“小毛”在“小猫”之前。将基础词库“小猫”从 5072 调至 5400 的试验中，15 个样例 Top1 均命中；该分数是针对这一同音竞争的排序调整，不是实测词频。生成脚本保留该覆盖值。其他分类未因小样本整体调权。

另在隔离配置中开启人工智能、医疗、法律、财务会计分类及医疗、法律专业基础词库复测，15 个样例仍均为 Top1；该结果仅覆盖这 15 个样例。
