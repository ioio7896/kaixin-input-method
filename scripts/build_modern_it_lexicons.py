#!/usr/bin/env python3
"""Build modern AI, software, internet-product and framework lexicons."""

from __future__ import annotations

import argparse
import math
from pathlib import Path

from opencc import OpenCC
from pypinyin import Style, lazy_pinyin
from wordfreq import top_n_list


ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "data_sources" / "lexicon_fragments" / "zh-ext"
T2S = OpenCC("t2s")
POOL_SIZE = 500_000
SIZE = 5_000

MIXED_CODES = {
    "AIGC": "aigc", "ChatGPT": "chat gpt", "DeepSeek": "deep seek", "GPU": "gpu",
    "Wi-Fi": "wi fi", "C++": "c", "B站": "b zhan", "UP主": "up zhu",
    "GitHub": "git hub", "Windows 11": "windows", "Linux": "linux",
    "Docker": "docker", "Kubernetes": "kubernetes", "API": "api", "SDK": "sdk",
    "Python": "python", "Java": "java", "JavaScript": "java script",
    "TypeScript": "type script", "Rust": "rust", "Go": "go", "React": "react",
    "Vue": "vue", "Angular": "angular", "Spring": "spring", "Django": "django",
    "Flask": "flask", "TensorFlow": "tensor flow", "PyTorch": "py torch",
    "NumPy": "num py", "OpenAI": "open ai", "Claude": "claude", "Gemini": "gemini",
    "Kotlin": "kotlin", "Swift": "swift", "Dart": "dart", "PHP": "php", "Ruby": "ruby",
    "C#": "c", "SQL": "sql", "HTML": "html", "CSS": "css", "Shell": "shell",
    "ReactNative": "react native", "Svelte": "svelte", "Next.js": "next js", "Nuxt": "nuxt",
    "Node.js": "node js", "Express": "express", "FastAPI": "fast api", "Laravel": "laravel",
    ".NET": "net", "Unity": "unity", "Unreal": "unreal", "Pandas": "pandas",
    "ScikitLearn": "scikit learn", "Jupyter": "jupyter", "CUDA": "cuda", "OpenGL": "open gl",
    "Qt": "qt", "Electron": "electron", "Webpack": "webpack", "Vite": "vite", "Babel": "babel",
    "ESLint": "eslint", "Prettier": "prettier", "Git": "git", "GitLab": "git lab", "Gitee": "gitee",
    "Jenkins": "jenkins", "Maven": "maven", "Gradle": "gradle", "npm": "npm", "Yarn": "yarn",
    "Cargo": "cargo", "CMake": "cmake", "Redis": "redis", "MySQL": "my sql", "PostgreSQL": "postgre sql",
    "SQLite": "sql lite", "MongoDB": "mongo db", "Elasticsearch": "elastic search", "Kafka": "kafka", "Nginx": "nginx",
    "TikTok": "tik tok", "YouTube": "you tube", "Instagram": "instagram",
    "Facebook": "facebook", "X平台": "x ping tai", "AppStore": "app store",
}

AI_SEEDS = """
生成式人工智能|大语言模型|多模态|智能体|提示词|文生图|文生视频|检索增强生成|向量数据库|知识库问答|云端推理|模型训练|模型微调|模型评测|模型部署|机器学习|深度学习|强化学习|迁移学习|联邦学习|神经网络|卷积神经网络|循环神经网络|生成对抗网络|自然语言处理|计算机视觉|语音识别|语音合成|图像识别|目标检测|语义理解|文本分类|情感分析|问答系统|推荐系统|搜索增强|知识图谱|数据标注|训练数据|推理服务|推理加速|推理引擎|训练平台|模型服务|模型仓库|模型权重|开源模型|基础模型|预训练模型|通用人工智能|人工智能应用|人工智能助手|智能问答|智能搜索|智能客服|智能推荐|智能驾驶|智能制造|智能医疗|智能教育|计算机智能|机器视觉|大模型应用|大模型训练|大模型推理|大模型评测|大模型安全|大模型幻觉|上下文窗口|上下文学习|长文本处理|提示词工程|提示词模板|提示词注入|提示词优化|思维链|思维链推理|多轮对话|对话模型|聊天机器人|数字人|虚拟人|语音助手|图像生成|图像编辑|视频生成|三维生成|音乐生成|代码生成|代码补全|自动编程|程序合成|检索增强|知识增强|语料库|训练语料|特征工程|特征向量|向量检索|相似度搜索|模型蒸馏|模型剪枝|参数高效微调|低秩适配|监督学习|无监督学习|半监督学习|零样本学习|少样本学习|强化智能|深度神经网络|注意力机制|自注意力|变换器模型|扩散模型|语言模型|视觉语言模型|多模态模型|端到端模型|大规模预训练|模型可解释性|算法偏见|数据隐私|人工智能治理|人工智能伦理|安全对齐|模型对齐|内容审核|深度合成|合成数据|人工智能芯片|算力中心|算力网络|算力调度|异构计算|边缘智能|端侧智能|AI应用|AIGC|ChatGPT|DeepSeek|OpenAI|Claude|Gemini|GPU
""".split("|")

SOFTWARE_SEEDS = """
云原生|云计算|云服务|云平台|云基础设施|云端应用|容器化|容器编排|容器镜像|容器集群|微服务|服务网格|无服务器|函数计算|弹性计算|虚拟化|虚拟机|操作系统|文件系统|数据库|关系数据库|非关系数据库|分布式数据库|时序数据库|缓存系统|消息队列|数据仓库|数据湖|数据中台|数据治理|数据备份|数据恢复|网络协议|网络安全|网络架构|负载均衡|反向代理|域名解析|服务发现|配置中心|注册中心|持续集成|持续交付|自动化部署|自动化测试|开发工具|软件开发|软件工程|系统架构|应用架构|技术架构|后端开发|前端开发|全栈开发|客户端开发|移动开发|接口开发|接口文档|应用程序接口|软件开发工具包|版本控制|代码仓库|代码审查|代码质量|代码规范|编译构建|编译器|解释器|运行时环境|依赖管理|包管理器|插件系统|模块化|面向对象|函数式编程|并发编程|异步编程|多线程|分布式系统|高可用|高并发|容灾备份|故障恢复|监控告警|日志分析|链路追踪|性能优化|压力测试|安全审计|身份认证|权限管理|访问控制|单点登录|数据加密|密钥管理|零信任|隐私计算|安全漏洞|漏洞修复|防火墙|入侵检测|终端安全|应用安全|供应链安全|软件供应链|开源软件|开源协议|软件许可证|技术文档|用户手册|产品文档|需求分析|产品设计|交互设计|用户体验|测试环境|生产环境|开发环境|灰度发布|版本发布|热更新|系统升级|Windows 11|Linux|Docker|Kubernetes|API|SDK|GPU|Wi-Fi|GitHub|C++
""".split("|")

INTERNET_SEEDS = """
小红书|哔哩哔哩|拼多多|飞书|钉钉|微信|微信公众号|朋友圈|微博|抖音|快手|淘宝|京东|支付宝|百度|腾讯|阿里巴巴|网易|字节跳动|美团|饿了么|知乎|豆瓣|虎扑|贴吧|豆瓣小组|视频号|直播间|直播带货|短视频|长视频|网络直播|电商平台|电子商务|即时零售|社交平台|内容平台|游戏平台|音乐平台|视频网站|在线教育|在线办公|远程办公|移动支付|网络支付|扫码支付|数字钱包|网购平台|购物网站|商品详情|购物车|订单信息|订单状态|物流信息|快递查询|售后服务|退款申请|退货退款|优惠券|满减活动|会员权益|积分兑换|直播间互动|弹幕评论|粉丝关注|内容创作者|网络主播|带货主播|视频博主|知识付费|在线课程|订阅服务|热搜榜单|热门话题|网络热词|社区讨论|用户评论|点赞收藏|转发分享|私信消息|关注列表|推荐内容|个性化推荐|搜索结果|平台规则|账号安全|实名认证|隐私设置|消息通知|应用商店|手机应用|移动应用|小程序|公众号|企业微信|微信支付|支付宝转账|抖音商城|淘宝直播|京东物流|拼多多百亿补贴|小红书笔记|哔哩哔哩视频|飞书文档|钉钉办公|B站|UP主|TikTok|YouTube|Instagram|Facebook|X平台|AppStore
""".split("|")

FRAMEWORK_SEEDS = """
Python|Java|JavaScript|TypeScript|Rust|Go|C++|Kotlin|Swift|Dart|PHP|Ruby|C#|SQL|HTML|CSS|Shell|R语言|React|Vue|Angular|Svelte|Next.js|Nuxt|Node.js|Express|Spring|SpringBoot|Django|Flask|FastAPI|Laravel|.NET|Unity|Unreal|TensorFlow|PyTorch|NumPy|Pandas|ScikitLearn|Jupyter|CUDA|OpenGL|Qt|Electron|Webpack|Vite|Babel|ESLint|Prettier|Git|GitHub|GitLab|Gitee|Docker|Kubernetes|Jenkins|Maven|Gradle|npm|Yarn|Cargo|CMake|Redis|MySQL|PostgreSQL|SQLite|MongoDB|Elasticsearch|Kafka|Nginx|Linux内核|安卓开发|Android开发|iOS开发|小程序开发|跨平台开发|移动端开发|前端框架|后端框架|开发框架|编程语言|程序设计|数据结构|算法设计|设计模式|软件架构|接口设计|组件库|UI框架|网络编程|并发编程|异步编程|函数式编程|面向对象编程|单元测试|集成测试|自动化测试|持续集成|持续部署|版本控制|代码管理|代码托管|代码审查|代码格式化|依赖注入|消息队列|微服务架构|服务端渲染|前后端分离|数据库连接|数据库驱动|云原生开发|容器编排|机器学习框架|深度学习框架|自然语言处理|计算机视觉|大语言模型|生成式人工智能|API接口|SDK开发|GPU编程|C++编程|Java开发|Python开发|JavaScript开发|Rust开发|Go语言开发
""".split("|")

CATEGORIES = {
    "ai_and_machine_learning": (AI_SEEDS, ("人工智能", "机器学习", "深度学习", "模型", "训练", "推理", "生成", "智能", "神经", "语言", "视觉", "语音", "提示词", "向量", "数据标注", "多模态")),
    "software_and_cloud": (SOFTWARE_SEEDS, ("软件", "云", "容器", "系统", "数据库", "网络", "服务", "开发", "部署", "安全", "代码", "数据", "架构", "接口", "版本", "编译", "运行")),
    "internet_products": (INTERNET_SEEDS, ("小红书", "哔哩", "拼多多", "飞书", "钉钉", "微信", "微博", "抖音", "淘宝", "京东", "支付", "直播", "视频", "平台", "电商", "社交", "用户", "内容", "账号", "评论", "订单")),
    "programming_frameworks": (FRAMEWORK_SEEDS, ("Python", "Java", "JavaScript", "TypeScript", "Rust", "Go", "编程", "开发", "框架", "代码", "算法", "数据库", "测试", "部署", "接口", "组件", "语言", "前端", "后端", "微服务", "容器")),
}


def is_cjk_phrase(phrase: str) -> bool:
    return 2 <= len(phrase) <= 12 and all("\u4e00" <= char <= "\u9fff" for char in phrase) and T2S.convert(phrase) == phrase


def code_for(phrase: str) -> str:
    if phrase in MIXED_CODES:
        return MIXED_CODES[phrase]
    values = lazy_pinyin(phrase, style=Style.NORMAL, errors="default")
    if len(values) != len(phrase) or any(not value.isascii() or not value.isalpha() for value in values):
        raise ValueError(f"invalid pinyin for {phrase!r}: {values!r}")
    return " ".join(values)


def candidates() -> list[tuple[str, int]]:
    seen: set[str] = set()
    out: list[tuple[str, int]] = []
    for rank, phrase in enumerate(top_n_list("zh", POOL_SIZE), 1):
        if phrase not in seen and is_cjk_phrase(phrase):
            seen.add(phrase)
            out.append((phrase, rank))
    return out


def score(phrase: str, rank: int, keywords: tuple[str, ...]) -> int:
    return rank - 60_000 * sum(phrase.count(keyword) for keyword in keywords)


def write(path: Path, title: str, seeds: list[str], keywords: tuple[str, ...], pool: list[tuple[str, int]]) -> None:
    selected: list[tuple[str, int]] = []
    seen: set[str] = set()
    for phrase in seeds:
        phrase = phrase.strip()
        if not phrase or phrase in seen or (phrase not in MIXED_CODES and not is_cjk_phrase(phrase)):
            continue
        selected.append((phrase, 1))
        seen.add(phrase)
    ranked = sorted(((phrase, rank) for phrase, rank in pool if phrase not in seen), key=lambda item: (score(item[0], item[1], keywords), item[1], item[0]))
    selected.extend(ranked[: max(0, SIZE - len(selected))])
    if len(selected) != SIZE:
        raise RuntimeError(f"{path.name}: only generated {len(selected)} rows")
    rows = []
    for phrase, rank in selected:
        rows.append(f"{phrase}\t{code_for(phrase)}\t{max(1_000, int(round(10_000_000 / math.sqrt(rank))))}")
    path.write_text(
        f"# 开心输入法扩展词库：{title}\n"
        "# 格式：词语<TAB>全拼或混合输入码<TAB>权重；混合实体保留 ASCII 大小写和符号。\n"
        "# Source: wordfreq 中文频率排序 + 项目维护的现代技术实体与产品名称。\n"
        + "\n".join(rows) + "\n",
        encoding="utf-8",
        newline="\n",
    )
    print(path.name, len(rows))


def main() -> None:
    parser = argparse.ArgumentParser()
    parser.parse_args()
    pool = candidates()
    OUT.mkdir(parents=True, exist_ok=True)
    for name, (seeds, keywords) in CATEGORIES.items():
        write(OUT / f"{name}.txt", name, seeds, keywords, pool)
    from merge_zh_ext_lexicons import merge_lexicons

    merge_lexicons()


if __name__ == "__main__":
    main()
