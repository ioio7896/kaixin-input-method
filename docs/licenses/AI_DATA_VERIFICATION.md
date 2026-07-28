# AI 辅助项目数据核验记录

status: verified

核验日期：2026-07-27  
核验负责人：项目作者  
核验脚本：`python scripts/verify_ai_generated_data.py`

## 历史记录的限制

项目作者确认，这 14 个文件均由其在本机使用 OpenAI ChatGPT 辅助生成或整理，并由其
决定纳入项目；这里的“本机”指从本机访问 ChatGPT 服务，不表示使用本地部署模型。
历史聊天记录中的原始提示、精确模型版本、具体客户端（网页、桌面客户端或 API）、
首次生成时间和当时是否启用联网检索已经无法完整恢复。本文不对这些未知项作推测。

因此，本次核验没有声称能够按历史过程逐字节重放。下列重建任务说明是当前冻结数据的
复核规范；固定 SHA-256 才是可复现地标识获准发布版本的依据。

## 重建后的明确任务说明

所有任务共用以下约束：只输出 UTF-8 数据；词库采用“词语、无声调且以空格分隔的拼音、
正整数权重”三个制表符分隔字段；多音词表采用前两个字段；去除完全重复的
“词语 + 拼音”组合；不复制百科、新闻、地图或商业词库中的介绍性文字。

1. 繁简映射：整理单字级简体到繁体映射，保留一简对多繁的可能，以从 0 连续递增的
   `sort_order` 写入 SQLite 表 `s2t_chars(sort_order, simplified, traditional)`。
2. 全国行政区划：整理 333 个地级行政区名称和 2024 年县级行政区常用简称；核对名称、
   去重并标注标准普通话拼音。
3. 杭州地铁：整理 262 个不重复站名并标注标准普通话拼音。
4. 杭州扩展词库：按行政区划、商圈企业、饮食文化、景点生活、本地文化、新地名、
   公共服务和交通八类整理适合日常输入的专名；完整名称优先，歧义简称降低权重。
5. 项目基础数据：整理基础常用候选及多音词纠音；允许同一词语因规范异读出现多行，
   但不允许完全相同的“词语 + 拼音”重复。

## 服务条款依据

本次核验参考 2026-01-01 生效的 OpenAI《使用条款》：
<https://openai.com/zh-Hans-CN/policies/terms-of-use/>。条款说明，在适用法律允许的
范围内，用户拥有输出；同时输出可能不唯一，用户仍负责评估输出的准确性和适用性。
因此，ChatGPT 输出权利说明是发布依据之一，但不是事实准确性的替代证明。

## 事实核验方法和边界

- 全国行政区划以民政部行政区划与地名公共服务资料为权威核对入口：
  <https://www.mca.gov.cn/>、<https://dmfw.mca.gov.cn/>。
- 杭州行政区划和新地名以杭州市人民政府及杭州市民政局公开资料为核对入口：
  <https://www.hangzhou.gov.cn/>。
- 杭州地铁站名以杭州地铁运营服务页面和运营线网图为核对入口：
  <https://www.hzmetro.com/service_323.aspx>。
- 拼音、列数、空字段、重复组合、权重、文件哈希和 SQLite 完整性由核验脚本全量检查；
  代表性杭州词条另由 `pinyin-ime/tests/hangzhou_city_lexicon.rs` 自动测试。
- 商圈、企业、医院、学校等现实名称可能在发布后更名或停用；`verified` 表示这里记录的
  冻结版本来源与验收过程可复核，不表示全部现实状态永久有效。后续修改任何文件都必须
  更新哈希、核验日期并重新运行检查。

## 冻结文件清单

| 文件 | 数据行数 | SHA-256 |
| --- | ---: | --- |
| `pinyin-ime/data/s2t_chars.sqlite` | 2714 | `f09b4aa90b1569a22c49d222e7930bf98406b81a1a12c581a6b67f9e3915cc3a` |
| `lexicon/base/china_prefecture_level_admin_333.txt` | 333 | `7c42ec07dff26f8c678f5a79e3409bd9dc59f0335a1375b0a071cb382244b982` |
| `lexicon/base/county_admin_short_names_2024.txt` | 2770 | `3a8b07c00ea23ccce40fc66be29f00bd6f6b3f68ab58641542e9dd44d5fb7ab6` |
| `lexicon/base/hangzhou_metro_stations_262.txt` | 262 | `d944295047ac351ef1b48a277fcd352e51a27ebfc4da5d747f184d80464cd919` |
| `lexicon/ext/hangzhou_admin.txt` | 300 | `ddc8aebc95b90b616ca94df1e5bdebdcd50756f9ecf9d67aedfdccc187fe0251` |
| `lexicon/ext/hangzhou_business.txt` | 85 | `98a841788c5faf9a37b78a60265faf6583928b3c645af080e6c9be4a768e87d1` |
| `lexicon/ext/hangzhou_food_culture.txt` | 75 | `dc5b1133ec375331242383f0ab784deaaf23fb862b081b5d7543253ad29764ce` |
| `lexicon/ext/hangzhou_landmarks_life.txt` | 165 | `97f6d2ecb7ac93cc27e15fde821adc56b30a4cb87005fd49b90272fd40a7cbf6` |
| `lexicon/ext/hangzhou_local_culture.txt` | 65 | `208ee6393998fa371eccedea131b5fa168906c0fa8b768bd917f7880f808cc6f` |
| `lexicon/ext/hangzhou_new_places.txt` | 131 | `bbfb433ae79d2c9b12a423e5b78108a413224f2e625a125347c311078c05a5d3` |
| `lexicon/ext/hangzhou_public_services.txt` | 165 | `a6309880503dccbafaa5e135a004d6c6e70179b431f06e25e1174b9d18e39208` |
| `lexicon/ext/hangzhou_transport.txt` | 145 | `0609c01c369e1ef9e6fcfa366a273d7e8d7ba8eca0791dffcdd34565d0d6dc39` |
| `data_sources/kaixin/common_phrases.tsv` | 46 | `d4890760141edacfaa443d1ef6c250b75f6d7b9cfb73ae6914e473ead9d07328` |
| `data_sources/kaixin/polyphone_corrections.tsv` | 145 | `b40830a7560d63feeaeeda79c2e5f15cdda35078100d918a543e7c2dff60be6d` |

`common_phrases.tsv` 中“简略”和“暴虐”各有两种拼音，属于有意保留的异读，不是完全
重复。其余文本文件的词语字段均无重复；所有文件的“词语 + 拼音”组合均无重复。
