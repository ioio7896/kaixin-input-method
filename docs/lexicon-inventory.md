# 当前词库清单

由 `python scripts/lexicon_quality.py --apply` 按当前运行时输出生成；冻结导入报告保留历史数据。

扩展分类：26 个；默认开启 12 个。

| 文件 | 名称 | 分组 | 默认开启 | 条目数 | 分数范围 |
|---|---|---|---|---:|---|
| daily_communication.txt | 聊天与社交 | 日常表达 | 是 | 4132 | 1430—8200 |
| category_life_daily.txt | 日常生活与家居 | 日常表达 | 是 | 1145 | 56—5800 |
| category_dining.txt | 美食烹饪与餐饮 | 日常表达 | 是 | 4765 | 91—6200 |
| category_shopping.txt | 购物物流与支付 | 日常表达 | 是 | 1769 | 72—6200 |
| category_travel.txt | 出行交通与旅游 | 日常表达 | 是 | 1364 | 229—6200 |
| category_office.txt | 办公协作与职场 | 办公与办事 | 是 | 2751 | 2010—6200 |
| category_government.txt | 政务社保与应急 | 办公与办事 | 是 | 420 | 2000—6200 |
| category_education.txt | 教育考试与阅读 | 日常表达 | 是 | 2271 | 1424—6200 |
| category_games.txt | 游戏文娱与运动 | 日常表达 | 是 | 1586 | 658—6200 |
| animals.txt | 动物与宠物 | 日常表达 | 是 | 621 | 1900—5200 |
| category_brands.txt | 品牌与应用 | 名称与实体 | 是 | 2549 | 2160—6200 |
| geography_admin.txt | 行政区划与地名 | 地区词库 · 全国与世界 | 是 | 5667 | 1500—6000 |
| hangzhou_local.txt | 杭州本地生活与地理 | 地区词库 · 杭州 | 否 | 1674 | 175—6200 |
| technology.txt | 科技编程与人工智能 | 科技与开发 | 否 | 2188 | 4600—7200 |
| category_medical.txt | 医疗健康与临床 | 专业领域 | 否 | 704 | 1654—6200 |
| medicine.txt | 常见药物与护理 | 药物名称 | 否 | 545 | 2400—5200 |
| category_finance.txt | 金融财会与保险 | 专业领域 | 否 | 1576 | 118—6200 |
| category_law.txt | 法律与合同 | 专业领域 | 否 | 268 | 313—6200 |
| category_science.txt | 数理化生与科研 | 专业领域 | 否 | 1284 | 584—6200 |
| category_engineering.txt | 建筑制造与能源环保 | 专业领域 | 否 | 973 | 209—6200 |
| category_humanities.txt | 文史哲地与音乐 | 专业领域 | 否 | 891 | 117—6200 |
| category_idioms.txt | 成语与固定表达 | 成语与固定表达 | 否 | 20905 | 2373—5416 |
| people_names.txt | 人物姓名 | 名称与实体 | 否 | 8000 | 200—1200 |
| category_proper_organizations.txt | 机构与组织名称 | 名称与实体 | 否 | 1114 | 2135—3850 |
| category_common_three.txt | 三字精选补充 | 通用补充 | 否 | 25004 | 2000—5263 |
| category_common_four.txt | 四字及以上精选补充 | 通用补充 | 否 | 25013 | 2000—6103 |

分数为来源相关的候选排序优先级，不代表实测用户词频；分类开关不影响 core/zh 常开层。
