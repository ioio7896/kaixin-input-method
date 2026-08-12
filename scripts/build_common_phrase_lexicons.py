#!/usr/bin/env python3
"""Build the four modern short-phrase lexicons.

The source ordering comes from ``wordfreq``'s Chinese frequency list.  A
small, deterministic template tail supplies natural 5--8 character workflow
phrases that are too sparse to appear as standalone entries in wordfreq.
The output uses the native ``phrase<TAB>pinyin<TAB>weight`` format.
"""

from __future__ import annotations

import argparse
import math
from pathlib import Path

from opencc import OpenCC
from pypinyin import Style, lazy_pinyin
from wordfreq import top_n_list, zipf_frequency


ROOT = Path(__file__).resolve().parents[1]
BASE = ROOT / "lexicon" / "zh"
POOL_SIZE = 500_000
BLOCKED = ("他妈", "你妈", "妈逼", "傻逼", "煞笔", "操你", "艹你", "狗日", "王八蛋")
T2S = OpenCC("t2s")

CHAT_SEEDS = [phrase.strip() for phrase in """
辛苦了|稍等一下|麻烦你了|不客气|怎么回事|谢谢你了|谢谢大家|不用谢|没关系的|好的好的|收到消息|我知道了|明白了|了解一下|等我一下|马上回来|回头再说|有空聊聊|晚安好梦|早上好呀|晚上好呀|周末愉快|节日快乐|生日快乐|新年快乐|一路顺风|注意安全|保重身体|最近怎么样|你吃饭了吗|今天还好吗|现在方便吗|可以聊聊吗|方便接电话吗|有什么事吗|发生什么事|这是怎么了|到底怎么回事|你觉得呢|你怎么看|我也是这么想的|我不太清楚|我也不知道|先这样吧|那就这样吧|回头联系|改天再聊|有事联系我|别着急慢慢来|不用担心|一切都会好的|祝你好运|加油加油|哈哈哈哈|笑死我了|太真实了|真的假的呀|不会吧不会吧|我已经到了|我快到了|我在路上|马上就到|稍后回复你|晚点再说|等一下再说|你先忙吧|你先休息吧|早点休息|好久不见|很高兴认识你|认识你很开心|感谢你的帮助|麻烦看一下|帮我看一下|可以帮个忙吗|有空帮我看看|方便发给我吗|请你确认一下|记得回复我|别忘了回复|看到请回复|先谢谢你了|辛苦你帮忙|麻烦你抽空|打扰一下哈|抱歉打扰了|不好意思啊|对不起啦|没事没事|都过去了|问题不大|别放在心上|我理解你的心情|听起来不错|这个主意不错|感觉还不错|我觉得可以|我觉得不行|你说得对|说得很有道理|我同意你的看法|我不这么认为|我们再商量一下|让我想一想|我考虑一下|容我想想|你慢慢说|请继续说下去|能再说一遍吗|我没有听清楚|可以再解释一下吗|你说的是什么意思|你指的是这个吗|请问怎么操作|怎么使用这个|这个怎么解决|我来试试看|我试一下看看|已经解决了|现在可以用了|还是不行啊|又出问题了|稍微有点问题|我来处理一下|等我处理完|先把事情说清楚|具体是什么情况|你现在在哪里|什么时候有空|明天见面吗|下次再约吧|改天一起吃饭|有机会再见|到家了吗|到了告诉我|路上注意安全|开车慢一点|早点回家休息|今天辛苦了|工作加油啊|学习加油啊|考试顺利啊|面试顺利啊|祝你工作顺利|祝你天天开心|希望你能理解|希望一切顺利|愿你每天开心|新的一天加油|今天也要开心|记得照顾自己|别忘了吃饭|多喝一点水|早点睡觉吧|做个好梦吧
""".split("|") if phrase.strip()]

OFFICE_SEEDS = [phrase.strip() for phrase in """
项目进展顺利|项目已经启动|项目正在进行|项目按计划推进|项目进度正常|项目需要跟进|项目需要复盘|项目情况汇报|项目阶段总结|项目风险评估|项目预算审批|项目需求确认|项目方案讨论|项目计划安排|项目工作计划|项目实施方案|项目验收通过|项目按时完成|项目延期申请|项目交付时间|工作安排如下|工作已经完成|工作正在进行|工作进度汇报|工作内容确认|工作计划调整|工作重点说明|工作任务分配|工作需要配合|工作及时跟进|工作认真负责|工作效率提升|部门工作安排|部门之间协调|部门会议通知|部门负责人确认|部门年度总结|部门季度计划|会议时间调整|会议安排如下|会议内容确认|会议材料准备|会议纪要整理|会议结果汇报|会议已经结束|会议按时召开|请准时参加会议|请提前准备材料|请及时回复邮件|请查收邮件附件|请查看相关文件|请确认收到邮件|请大家知悉|请各位注意查收|请按照要求办理|请在今天完成|请于明天提交|请尽快处理一下|请及时反馈意见|请提供详细资料|请补充相关信息|请核对数据内容|请确认最终版本|请安排人员参加|请做好会议准备|请提前通知大家|请抄送相关人员|请回复处理结果|邮件已经发送|邮件附件如下|邮件内容确认|邮件需要回复|邮件发送成功|邮件发送失败|附件已经上传|附件请查收|附件内容如下|文件已经完成|文件正在审核|文件需要修改|文件版本更新|文件格式错误|文件内容确认|文件已经归档|资料已经准备好|资料需要补充|资料请及时提供|资料内容有误|资料已经提交|材料准备完成|材料需要盖章|材料需要审核|材料已经收到|材料请尽快提交|数据已经更新|数据需要核对|数据分析报告|数据统计结果|数据导出完成|数据安全检查|数据异常情况|数据来源说明|数据请及时同步|客户需求确认|客户反馈意见|客户服务支持|客户关系维护|客户信息更新|客户已经确认|客户需要跟进|客户问题处理|客户满意度调查|供应商联系确认|合同已经签署|合同需要审核|合同条款确认|合同即将到期|合同审批流程|合同附件准备|报价方案确认|报价已经提交|报价需要调整|预算已经批准|预算执行情况|发票已经开具|发票信息核对|费用报销申请|费用报销完成|财务数据核对|付款申请提交|付款已经完成|收款信息确认|审批流程如下|审批已经通过|审批需要补充材料|流程需要优化|流程已经更新|流程执行情况|制度文件发布|制度内容说明|通知已经发布|通知内容如下|通知请及时查看|培训时间安排|培训材料准备|培训计划确认|招聘需求确认|面试时间安排|入职手续办理|离职手续办理|考勤记录核对|请假申请提交|加班申请审批|年终总结会议|季度工作总结|月度工作计划|年度目标完成|目标需要调整|指标完成情况|绩效考核结果|工作日报提交|工作周报发送|工作月报汇总|进度及时同步|进展情况说明|后续工作安排|下一步工作计划|后续安排如下|问题清单整理|问题已经解决|问题需要跟进|问题原因分析|问题处理结果|风险问题提醒|风险控制措施|风险已经解除|需要进一步确认|需要领导审批|需要大家配合|需要统一安排|需要重新检查|建议尽快处理|建议召开会议|建议修改方案|建议补充说明|方案已经确定|方案需要调整|方案内容如下|方案评审通过|版本已经发布|版本更新说明|系统运行正常|系统需要升级|系统故障排查|系统权限申请|账号权限开通|账号信息确认|密码需要重置|网络连接异常|服务已经恢复|服务暂时不可用|平台功能更新|平台使用说明|上线时间确定|上线前检查|上线后观察|测试结果通过|测试环境准备|测试数据清理|测试问题反馈|发布流程确认|发布计划安排|上线计划如下|请勿外传文件|内部资料保密|仅供内部使用|未经允许不得转载|请遵守工作纪律|感谢大家配合|辛苦大家配合|谢谢大家支持|感谢您的理解|如有问题请联系|如有疑问请反馈|后续会及时通知|具体安排另行通知|最终结果另行通知|以上请知悉|特此通知|请审批处理|请领导审阅|请批示意见|烦请审核|烦请查收|烦请回复|敬请知悉|请予以确认|请按时完成任务|请做好记录|请保存相关文件|请勿修改内容|请勿删除数据|请注意查收邮件|请及时更新进度|请同步最新信息
""".split("|") if phrase.strip()]

# These are deliberately ordinary collocations rather than a second idiom
# list.  They fill the small long-phrase tail absent from wordfreq's token list.
LONG_SUBJECTS = "我们|大家|本次活动|这项工作|当前项目|相关部门|项目团队|系统管理员|客户团队|所有人员|现场工作人员|后续安排|当前任务|新的计划|具体情况|最终结果|工作进度|会议安排|申请材料|反馈信息".split("|")
LONG_ACTIONS = "已经完成|正在处理|需要确认|可以解决|值得关注|请及时查看|已经开始|正在等待|需要继续|可以直接|请大家注意|能够有效|已经做好|正在准备|需要尽快|应该认真|已经提交|正在审核|需要补充|可以按照".split("|")
LONG_OBJECTS = "相关工作|后续安排|具体情况|新的计划|所有问题|最终结果|下一步工作|资料内容|反馈信息|申请材料|使用方法|安全设置|服务内容|会议安排|时间地点|操作步骤|审核结果|工作进度|处理方式|项目计划|最新消息|实际情况|执行结果|具体要求|相关文件|当前状态|重要信息|详细内容|处理意见|工作任务".split("|")


def valid_phrase(phrase: str, lengths: range | tuple[int, ...]) -> bool:
    return (
        len(phrase) in lengths
        and all("\u4e00" <= char <= "\u9fff" for char in phrase)
        and T2S.convert(phrase) == phrase
        and not any(blocked in phrase for blocked in BLOCKED)
    )


def reading(phrase: str) -> str:
    values = lazy_pinyin(phrase, style=Style.NORMAL, errors="default")
    if len(values) != len(phrase) or any(not value.isascii() or not value.isalpha() for value in values):
        raise ValueError(f"cannot render pinyin for {phrase!r}: {values!r}")
    return " ".join(values)


def source_candidates() -> list[tuple[str, int]]:
    converter = OpenCC("t2s")
    seen: set[str] = set()
    out: list[tuple[str, int]] = []
    for rank, phrase in enumerate(top_n_list("zh", POOL_SIZE), 1):
        if (
            phrase not in seen
            and 3 <= len(phrase) <= 8
            and all("\u4e00" <= char <= "\u9fff" for char in phrase)
            and converter.convert(phrase) == phrase
            and not any(blocked in phrase for blocked in BLOCKED)
        ):
            seen.add(phrase)
            out.append((phrase, rank))
    return out


def weight(rank: int, scale: int = 9_000_000) -> int:
    return max(1_000, int(round(scale / math.sqrt(max(1, rank)))))


def make_rows(phrases: list[tuple[str, int]], scale: int) -> list[str]:
    rows: list[str] = []
    for phrase, rank in phrases:
        rows.append(f"{phrase}\t{reading(phrase)}\t{weight(rank, scale)}")
    return rows


def select_ranked(
    candidates: list[tuple[str, int]],
    size: int,
    lengths: range | tuple[int, ...],
    *,
    seeds: list[str] | None = None,
    scorer=None,
) -> list[tuple[str, int]]:
    by_phrase = {phrase: rank for phrase, rank in candidates}
    selected: list[tuple[str, int]] = []
    seen: set[str] = set()
    for phrase in seeds or []:
        if valid_phrase(phrase, lengths) and phrase not in seen:
            # Curated expressions are intentionally promoted above the raw
            # frequency rank so they occupy the first candidate page.
            selected.append((phrase, 1))
            seen.add(phrase)
    pool = [(phrase, rank) for phrase, rank in candidates if len(phrase) in lengths and phrase not in seen]
    if scorer is not None:
        pool.sort(key=lambda item: (scorer(item[0], item[1]), item[1], item[0]))
    selected.extend(pool[: max(0, size - len(selected))])
    if len(selected) != size:
        raise RuntimeError(f"only selected {len(selected)} phrases; need {size}")
    return selected


def long_tail_extras(existing: set[str], needed: int) -> list[tuple[str, int]]:
    extras: list[tuple[str, int]] = []
    rank = POOL_SIZE + 1
    # Reuse curated chat/office expressions before falling back to templates.
    # This keeps the generated tail human-readable even when wordfreq has too
    # few standalone 5--8 character tokens.
    for phrase in [*CHAT_SEEDS, *OFFICE_SEEDS]:
        if valid_phrase(phrase, range(5, 9)) and phrase not in existing:
            extras.append((phrase, rank))
            existing.add(phrase)
            rank += 1
            if len(extras) >= needed:
                return extras
    for action in LONG_ACTIONS:
        for obj in LONG_OBJECTS:
            # Action + object yields complete, reusable collocations such as
            # “已经完成相关工作” and “正在处理具体情况”.
            phrase = action + obj
            if valid_phrase(phrase, range(5, 9)) and phrase not in existing:
                extras.append((phrase, rank))
                existing.add(phrase)
                rank += 1
                if len(extras) >= needed:
                    return extras
    raise RuntimeError(f"long phrase template tail only produced {len(extras)} rows")


def category_score(phrase: str, rank: int, keywords: tuple[str, ...]) -> float:
    hits = sum(phrase.count(keyword) for keyword in keywords)
    # Keep frequency dominant, but reserve enough headroom for category terms.
    return rank - hits * 25_000


def write(path: Path, title: str, source: str, rows: list[str]) -> None:
    path.write_text(
        f"# 开心输入法原生词库：{title}\n"
        f"# Source: {source}\n"
        "# 格式：词语<TAB>全拼<TAB>权重；拼音不标声调。\n"
        + "\n".join(rows)
        + "\n",
        encoding="utf-8",
        newline="\n",
    )


def build() -> None:
    candidates = source_candidates()
    life_keywords = ("可以", "需要", "已经", "正在", "比较", "还是", "如果", "因为", "所以", "我们", "大家", "请", "没有", "能够", "应该", "不要", "不能", "这是", "那个", "通过", "对于", "关于", "相关", "时候", "问题", "情况", "工作", "事情", "时间", "以后", "之前", "目前", "最后", "一般", "可能", "希望", "觉得", "知道", "看到", "发现", "开始", "继续", "结束", "一点", "起来", "一下")
    four = select_ranked(
        candidates,
        5_000,
        (4,),
        seeds=["与此同时", "总的来说", "换句话说"],
        scorer=lambda p, r: category_score(p, r, life_keywords),
    )
    long_candidates = [(p, r) for p, r in candidates if 5 <= len(p) <= 8]
    long_existing = {p for p, _ in long_candidates}
    long_candidates.extend(long_tail_extras(long_existing, max(0, 5_000 - len(long_candidates))))
    long = select_ranked(
        long_candidates,
        5_000,
        range(5, 9),
        seeds=[],
        scorer=lambda p, r: category_score(p, r, life_keywords),
    )
    chat_keywords = ("谢谢", "辛苦", "稍等", "麻烦", "客气", "怎么", "什么", "哈哈", "收到", "明白", "晚安", "加油", "方便", "回复", "联系", "希望", "祝", "抱歉", "不好意思", "没事", "可以吗", "好吗")
    office_keywords = ("项目", "工作", "会议", "邮件", "文件", "资料", "材料", "数据", "客户", "合同", "预算", "审批", "部门", "安排", "提交", "审核", "进度", "报告", "方案", "系统", "通知", "任务", "计划", "发票", "版本", "测试", "发布")
    chat = select_ranked(candidates, 5_000, range(3, 9), seeds=CHAT_SEEDS, scorer=lambda p, r: category_score(p, r, chat_keywords))
    office = select_ranked(candidates, 5_000, range(3, 9), seeds=OFFICE_SEEDS, scorer=lambda p, r: category_score(p, r, office_keywords))
    write(BASE / "life_common_4char.txt", "5000 条通用四字短语", "wordfreq 中文频率排序 + 高频现代固定搭配", make_rows(four, 9_000_000))
    write(BASE / "life_common_phrases_5to8.txt", "5000 条五至八字通用短语", "wordfreq 中文频率排序 + 现代工作与生活固定搭配模板", make_rows(long, 8_000_000))
    write(BASE / "chat_common_phrases.txt", "5000 条聊天口语短语", "wordfreq 中文频率排序 + 常用聊天表达整理", make_rows(chat, 7_000_000))
    write(BASE / "office_common_phrases.txt", "5000 条办公沟通短语", "wordfreq 中文频率排序 + 常用办公表达整理", make_rows(office, 7_000_000))
    print("generated:", "four", len(four), "long", len(long), "chat", len(chat), "office", len(office))


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.parse_args()
    build()
