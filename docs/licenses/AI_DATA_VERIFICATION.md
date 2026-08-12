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
- 商圈、企业、医院、学校等现实名称可能在发布后更名或停用；`verified` 表示这里记录的
  冻结版本来源与验收过程可复核，不表示全部现实状态永久有效。后续修改任何文件都必须
  更新哈希、核验日期并重新运行检查。

## 冻结文件清单

| 文件 | 数据行数 | SHA-256 |
| --- | ---: | --- |
| `pinyin-ime/data/s2t_chars.sqlite` | 2714 | `f09b4aa90b1569a22c49d222e7930bf98406b81a1a12c581a6b67f9e3915cc3a` |
| `lexicon/zh-ext/china_prefecture_level_admin_333.txt` | 333 | `cf0c9a8322b253fa7b5218134238054ca947431bc43a47d3f2a526e17fb8ed67` |
| `lexicon/zh-ext/county_admin_short_names_2024.txt` | 2770 | `e370c5f6d9c3d1465852a89c2e43095407491267a6f9afbc8d9288e873a7a3cd` |
| `lexicon/zh-ext/hangzhou_metro_stations_262.txt` | 262 | `c59fd37d8889097f7dfa46f5a0ea8ed147edafa671083f5eac5500d338d8321b` |
| `lexicon/zh-ext/hangzhou_admin.txt` | 300 | `082e70434a94ed31820657c8a673d44b801f4e5c1a9aa90add5503cf105fe463` |
| `lexicon/zh-ext/hangzhou_business.txt` | 85 | `bbf6c97ba7a70c210302521a422da5f02f70fb4e8ceacc3a653f7c1bd8864e2d` |
| `lexicon/zh-ext/hangzhou_food_culture.txt` | 75 | `52fa56b9e6f6fcb4efe447fa13d2aa319353eb68a4bd6d7882aa35a2bd9691fe` |
| `lexicon/zh-ext/hangzhou_landmarks_life.txt` | 165 | `7b1758d0ccebaf53bcfe6bf3242b8e665edd0c005695f410779e0b0e4fd589c6` |
| `lexicon/zh-ext/hangzhou_local_culture.txt` | 65 | `9244048090e2759942537d9c99fd46cb3fece27d33db709f3deaa60a9ae574f4` |
| `lexicon/zh-ext/hangzhou_new_places.txt` | 131 | `c47fc6428fdc481d8389e5c0ce0657fb46fb782aad19c08fd22e07f8ad2b7e62` |
| `lexicon/zh-ext/hangzhou_public_services.txt` | 165 | `e25892eb12a1e83bc26e0e4e888e74258db28263a68a156edfc63df67a1b5dae` |
| `lexicon/zh-ext/hangzhou_transport.txt` | 145 | `5a6032ad99f69af1ebab90e075b5eac1bdf7c44b9a3a3031528ed0226ca8952a` |
| `data_sources/kaixin/common_phrases.tsv` | 46 | `d4890760141edacfaa443d1ef6c250b75f6d7b9cfb73ae6914e473ead9d07328` |
| `data_sources/kaixin/polyphone_corrections.tsv` | 145 | `b40830a7560d63feeaeeda79c2e5f15cdda35078100d918a543e7c2dff60be6d` |

`common_phrases.tsv` 中“简略”和“暴虐”各有两种拼音，属于有意保留的异读，不是完全
重复。其余文本文件的词语字段均无重复；所有文件的“词语 + 拼音”组合均无重复。
