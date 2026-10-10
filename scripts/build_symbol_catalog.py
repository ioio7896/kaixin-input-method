"""Generate the offline symbol keyboard catalog from Python's Unicode database.

Only assigned, visible, standalone characters are included. No network source or
font is bundled. Commit the generated TSV so builds do not depend on Python/UCD.
"""
from pathlib import Path
import unicodedata as ud

ROOT = Path(__file__).resolve().parents[1]
OUT = ROOT / "pinyin-ime/data/symbol_catalog.tsv"
rows = []
seen = set()


def add(group, text, name=None, aliases=""):
    if (group, text) in seen:
        return
    seen.add((group, text))
    rows.append((group, text, name or ud.name(text, text), aliases))


def chars(group, text):
    for c in text:
        if not c.isspace():
            add(group, c)


def block(group, first, last, predicate=lambda c, n: True):
    for cp in range(first, last + 1):
        c = chr(cp)
        name = ud.name(c, "")
        if name and ud.category(c)[0] in "LNPS" and predicate(c, name):
            add(group, c, name)


# The first page is deliberately useful for daily work and science.
common = [
    ("，", "逗号", "douhao"), ("。", "句号", "juhao"), ("、", "顿号", "dunhao"),
    ("；", "分号", "fenhao"), ("：", "冒号", "maohao"), ("？", "问号", "wenhao"),
    ("！", "叹号", "tanhao"), ("…", "省略号", "shengluehao"), ("—", "破折号", "pozhehao"),
    ("·", "间隔号", "jiangehao"), ("“", "左双引号", "yinhao"), ("”", "右双引号", "yinhao"),
    ("‘", "左单引号", "danyinhao"), ("’", "右单引号", "danyinhao"),
    ("（", "左括号", "kuohao"), ("）", "右括号", "kuohao"),
    ("【", "左方括号", "fangkuohao"), ("】", "右方括号", "fangkuohao"),
    ("《", "左书名号", "shuminghao"), ("》", "右书名号", "shuminghao"),
    ("±", "正负号", "zhengfuhao plus minus"), ("∓", "负正号", "fuzhenghao"),
    ("×", "乘号", "chenghao multiply"), ("÷", "除号", "chuhao divide"),
    ("≈", "约等于", "yuedengyu approx"), ("≠", "不等于", "budengyu"),
    ("≤", "小于等于", "xiaoyudengyu"), ("≥", "大于等于", "dayudengyu"),
    ("∞", "无穷大", "wuqiongda infinity"), ("√", "平方根 根号", "genhao sqrt"),
    ("∛", "立方根", "lifanggen"), ("∜", "四次方根", "sicifanggen"),
    ("∑", "求和", "qiuhe sum sigma"), ("∏", "连乘积", "lianchengji product"),
    ("∫", "积分", "jifen integral"), ("∬", "二重积分", "erchongjifen"),
    ("∭", "三重积分", "sanchongjifen"), ("∮", "曲线积分 围道积分", "quxianjifen"),
    ("∂", "偏导数", "piandaoshu partial"), ("∇", "梯度 纳布拉", "tidu nabla"),
    ("∈", "属于", "shuyu element"), ("∉", "不属于", "bushuyu"),
    ("⊆", "子集", "ziji subset"), ("⊇", "超集", "chaoji superset"),
    ("∪", "并集", "bingji union"), ("∩", "交集", "jiaoji intersection"),
    ("∅", "空集", "kongji empty set"), ("∀", "任意 全称量词", "renyi forall"),
    ("∃", "存在量词", "cunzai exists"), ("∴", "所以", "suoyi therefore"),
    ("∵", "因为", "yinwei because"), ("∝", "正比于", "zhengbiyu"),
    ("∠", "角", "jiao angle"), ("⊥", "垂直", "chuizhi"), ("∥", "平行", "pingxing"),
    ("≡", "恒等于", "hengdengyu"), ("≅", "全等", "quandeng"),
    ("→", "右箭头", "youjiantou arrow"), ("←", "左箭头", "zuojiantou arrow"),
    ("↑", "上箭头", "shangjiantou"), ("↓", "下箭头", "xiajiantou"),
    ("↔", "双向箭头", "shuangxiangjiantou"), ("⇒", "推出 蕴含", "tuichu implies"),
    ("⇔", "等价 当且仅当", "dengjia iff"), ("⇌", "可逆反应 化学平衡", "kenifanying"),
    ("°", "度", "du degree"), ("℃", "摄氏度", "sheshidu celsius"),
    ("℉", "华氏度", "huashidu fahrenheit"), ("Ω", "欧姆 大写欧米伽", "oumu omega omeiga"),
    ("μ", "微 小写缪", "wei mu miu"), ("α", "阿尔法", "alpha aerfa"),
    ("β", "贝塔", "beta beita"), ("γ", "伽马", "gamma gama"),
    ("δ", "德尔塔", "delta deer ta"), ("ε", "艾普西隆", "epsilon"),
    ("θ", "西塔", "theta xita"), ("λ", "拉姆达", "lambda lamuda"),
    ("π", "圆周率 派", "pi yuanzhoulv"), ("σ", "西格玛", "sigma xigema"),
    ("φ", "斐", "phi fei"), ("ω", "欧米伽", "omega omeiga"),
    ("Δ", "大写德尔塔 增量", "delta zengliang"),
    ("²", "平方 上标二", "pingfang shangbiao"), ("³", "立方 上标三", "lifang shangbiao"),
    ("₂", "下标二", "xiabiao"), ("₃", "下标三", "xiabiao"),
    ("㎡", "平方米", "pingfangmi"), ("‰", "千分号", "qianfenhao"),
    ("‱", "万分号", "wanfenhao"), ("¥", "人民币 日元", "renminbi yen"),
    ("€", "欧元", "ouyuan euro"), ("£", "英镑", "yingbang pound"),
    ("©", "版权", "banquan copyright"), ("®", "注册商标", "zhuceshangbiao"),
    ("™", "商标", "shangbiao trademark"), ("✓", "对勾", "duigou check"),
    ("✗", "叉号", "chahao cross"), ("★", "实心星", "shixinxing star"),
    ("☆", "空心星", "kongxinxing star"), ("①", "带圈数字一", "quanyi"),
]
for text, name, aliases in common:
    add("常用", text, name, aliases)

chars("标点括号", "，。、；：？！…—·“”‘’（）【】《》〈〉「」『』〔〕〖〗［］｛｝﹙﹚﹛﹜﹝﹞")
block("标点括号", 0x21, 0x7E, lambda c, n: ud.category(c)[0] in "PS")
block("标点括号", 0x2010, 0x205E)
block("标点括号", 0x3001, 0x303F, lambda c, n: ud.category(c)[0] in "PS")
block("标点括号", 0xFE10, 0xFE6B)
chars("数学运算", "+−±∓×÷=≠≈≡≤≥∞√∛∜∑∏∐∫∬∭∮∯∰∂∇∈∉∋∌∅∪∩⊂⊃⊆⊇∀∃∄∴∵∝∠⊥∥")
for a, b in [(0x2200, 0x22FF), (0x27C0, 0x27EF), (0x2980, 0x2AFF)]:
    block("数学运算", a, b)
block("箭头", 0x2190, 0x21FF)
block("箭头", 0x27F0, 0x27FF)
block("箭头", 0x2900, 0x297F)
block("箭头", 0x2B00, 0x2BFF, lambda c, n: "ARROW" in n)
block("箭头", 0x1F800, 0x1F8FF)

greek_names = [
    ("Αα", "阿尔法", "alpha aerfa"), ("Ββ", "贝塔", "beta beita"),
    ("Γγ", "伽马", "gamma gama"), ("Δδ", "德尔塔", "delta deerta"),
    ("Εε", "艾普西隆", "epsilon"), ("Ζζ", "泽塔", "zeta zeta"),
    ("Ηη", "伊塔", "eta yita"), ("Θθ", "西塔", "theta xita"),
    ("Ιι", "约塔", "iota yueta"), ("Κκ", "卡帕", "kappa kapa"),
    ("Λλ", "拉姆达", "lambda lamuda"), ("Μμ", "缪", "mu miu"),
    ("Νν", "纽", "nu niu"), ("Ξξ", "克西", "xi kexi"),
    ("Οο", "奥密克戎", "omicron aomikerong"), ("Ππ", "派 圆周率", "pi pai yuanzhoulv"),
    ("Ρρ", "柔", "rho rou"), ("Σσς", "西格玛", "sigma xigema"),
    ("Ττ", "陶", "tau tao"), ("Υυ", "宇普西隆", "upsilon"),
    ("Φφϕ", "斐", "phi fei"), ("Χχ", "希", "chi xi"),
    ("Ψψ", "普西", "psi puxi"), ("Ωω", "欧米伽 欧姆", "omega omeiga oumu"),
]
for pair, name, aliases in greek_names:
    for i, c in enumerate(pair):
        add("希腊字母", c, ("大写" if i == 0 else "小写") + name, aliases)
block("希腊字母", 0x370, 0x3FF, lambda c, n: "GREEK" in n)

units = "℃℉°′″KΩΩ℧µμÅÅℓℏℎ℉℃‰‱㏑㏒㏕㎎㎏㎜㎝㎞㎟㎠㎡㎢㎣㎤㎥㎦㎖㎗㎘㎧㎨㎩㎪㎫㎬㎐㎑㎒㎓㎔㎕㎛㎭㎮㎯㍱㍲㍳㍴㍵㍶"
chars("物理单位", units)
for text, name in [("m/s", "米每秒"), ("m/s²", "加速度 米每二次方秒"), ("N·m", "力矩 牛顿米"),
                   ("J/(mol·K)", "摩尔气体常数单位"), ("W/(m·K)", "热导率单位"),
                   ("kg/m³", "密度 千克每立方米"), ("kWh", "千瓦时"), ("mol/L", "摩尔每升")]:
    add("物理单位", text, name)
block("物理单位", 0x3300, 0x33FF, lambda c, n: "SQUARE" in n and "KATAKANA" not in n)

for text, name in [("H₂O", "水"), ("CO₂", "二氧化碳"), ("O₂", "氧气"), ("H₂", "氢气"),
                   ("N₂", "氮气"), ("O₃", "臭氧"), ("NH₃", "氨"), ("CH₄", "甲烷"),
                   ("H₂SO₄", "硫酸"), ("HNO₃", "硝酸"), ("HCl", "盐酸 氯化氢"),
                   ("NaOH", "氢氧化钠"), ("Ca(OH)₂", "氢氧化钙"), ("CaCO₃", "碳酸钙"),
                   ("Na₂CO₃", "碳酸钠"), ("NaHCO₃", "碳酸氢钠"), ("NaCl", "氯化钠"),
                   ("KMnO₄", "高锰酸钾"), ("C₂H₅OH", "乙醇"), ("CH₃COOH", "乙酸"),
                   ("C₆H₁₂O₆", "葡萄糖"), ("H⁺", "氢离子"), ("OH⁻", "氢氧根离子"),
                   ("NH₄⁺", "铵根离子"), ("SO₄²⁻", "硫酸根离子"), ("CO₃²⁻", "碳酸根离子"),
                   ("NO₃⁻", "硝酸根离子"), ("PO₄³⁻", "磷酸根离子"), ("Fe²⁺", "亚铁离子"),
                   ("Fe³⁺", "铁离子"), ("Cu²⁺", "铜离子"), ("Ca²⁺", "钙离子"),
                   ("e⁻", "电子"), ("⇌", "可逆反应 化学平衡"), ("→", "反应生成"),
                   ("↑", "气体生成"), ("↓", "沉淀"), ("Δ", "加热"), ("°", "标准态"),
                   ("₍s₎", "固态"), ("₍l₎", "液态"), ("₍g₎", "气态")]:
    add("化学", text, name)
chars("上标下标", "⁰¹²³⁴⁵⁶⁷⁸⁹⁺⁻⁼⁽⁾ⁿⁱ₀₁₂₃₄₅₆₇₈₉₊₋₌₍₎ₐₑₒₓₕₖₗₘₙₚₛₜ")
block("上标下标", 0x2070, 0x209F)
block("数字序号", 0x2150, 0x218B)
block("数字序号", 0x2460, 0x24FF)
block("数字序号", 0x2776, 0x2793)
block("数字序号", 0x1F100, 0x1F1AD)
block("数字序号", 0x3220, 0x32FF, lambda c, n: "CIRCLED" in n or "PARENTHESIZED" in n)
chars("货币", "$¢£¤¥€₹₽₩₺₴₱฿₫₪₦₭₮₲₵₸₼₾₿")
block("货币", 0x20A0, 0x20BF)
chars("音乐", "♩♪♫♬♭♮♯")
block("音乐", 0x1D100, 0x1D1FF)
block("音乐", 0x1D000, 0x1D0FF)
block("几何图形", 0x25A0, 0x25FF)
block("几何图形", 0x2B00, 0x2BFF, lambda c, n: "ARROW" not in n)
block("星号装饰", 0x2700, 0x2775)
block("星号装饰", 0x2794, 0x27BF)
block("制表方块", 0x2500, 0x259F)
block("技术符号", 0x2300, 0x23FF)
block("技术符号", 0x2100, 0x214F)
block("杂项符号", 0x2600, 0x26FF)
block("数学字母", 0x1D400, 0x1D7FF)
block("国际音标", 0x250, 0x2FF)
block("拉丁扩展", 0xC0, 0x24F)
block("俄文字母", 0x400, 0x45F)
block("日文假名", 0x3041, 0x3096)
block("日文假名", 0x30A1, 0x30FA)
block("全角字符", 0xFF01, 0xFF60)
for a, b in [(0x1F300, 0x1F64F), (0x1F680, 0x1F6FF), (0x1F900, 0x1F9FF), (0x1FA70, 0x1FAFF)]:
    block("表情图标", a, b, lambda c, n: "SKIN TONE" not in n)

# Apply Chinese and pinyin names to the same symbols in every category.
translations = {text: (name, aliases) for text, name, aliases in common}
for i, (group, text, name, aliases) in enumerate(rows):
    if text in translations:
        zh, keys = translations[text]
        rows[i] = (group, text, zh, f"{keys} {name} {aliases}".strip())

OUT.write_text(
    f"# Offline symbol keyboard; Unicode {ud.unidata_version}; generated by scripts/build_symbol_catalog.py\n"
    "# category<TAB>text<TAB>name<TAB>aliases\n"
    + "".join("\t".join(row) + "\n" for row in rows), encoding="utf-8"
)
print(f"{len(rows)} entries, {len(set(r[1] for r in rows))} distinct symbols, {len(set(r[0] for r in rows))} categories")
