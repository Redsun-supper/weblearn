// SPDX-License-Identifier: AGPL-3.0-or-later
// Copyright (C) 2026  Redsun-supper (大冬呱 / HR_RedSun)
// 数学模块 · 学科分支体系知识图谱（初中 → 高中）
//
// 本文件是 ES module，两条加载路径：
//   1) 主站接入后由 main.js 动态 import() 并调用 initMathApp(container)；
//   2) 本地直接打开 math.html 时，由页面内联的 <script type="module"> 引导启动。
// 样式通过 import.meta.url 拼出 math.css 的绝对地址注入（与 english 取 WASM 同理：
// 相对路径在「主站注入」场景下会解析到主站地址，必须用模块自身地址）。
//
// 数据与 modules/math/syllabus.md 一一对应，改动时请两边同步。
// 本文件保持 ES5 写法（var / function），仅使用 export 做模块导出。

// ---------- 分支数据 ----------

// 节点结构：
//   id     唯一标识（也用于搜索选中定位）
//   name   显示名
//   badge  角标文字（册别 / 学段），btype: 'book' | 'grade'
//   role   一句话定位（详情页导语）
//   core   核心内容（数组，详情页列表）
//   keys   关键概念（数组，详情页 chips）
//   ideas  思想方法（数组，详情页 chips，可省略）
//   kids   子节点（可省略，省略即叶子）
var DATA = {
    junior: {
        tab: '初中 · 七至九年级',
        heading: '三大领域 · 10 个分支',
        groups: [
            {
                id: 'j-algebra', name: '数与代数', badge: '领域一', btype: 'grade',
                role: '暗线是「用字母表示数」——完成从算术思维到代数思维的过渡。',
                kids: [
                    {
                        id: 'j-num-expr', name: '数与式', badge: '七—九年级', btype: 'grade',
                        role: '代数思维的地基，完成「算术 → 代数」的过渡。',
                        core: [
                            '有理数：相反数、绝对值、数轴、有理数运算',
                            '实数：平方根、立方根、无理数、实数与数轴一一对应',
                            '代数式：整式（加减乘除、乘法公式）、分式、二次根式',
                            '整式乘除与因式分解：提公因式法、公式法、十字相乘法'
                        ],
                        keys: ['数轴', '绝对值', '科学记数法', '乘法公式', '因式分解', '最简二次根式'],
                        ideas: ['数形结合', '化归思想']
                    },
                    {
                        id: 'j-equation', name: '方程与不等式', badge: '七—九年级', btype: 'grade',
                        role: '用代数工具刻画等量关系与不等关系，是初中代数的核心应用。',
                        core: [
                            '一元一次方程：等式性质、解法、实际应用（行程、工程、销售模型）',
                            '二元一次方程组：代入消元、加减消元',
                            '一元二次方程：配方法、公式法、因式分解法、判别式、韦达定理',
                            '分式方程：增根的检验',
                            '一元一次不等式（组）的解法与数轴表示'
                        ],
                        keys: ['方程的解', '消元思想', '判别式 Δ', '解集', '数轴表示法'],
                        ideas: ['方程思想', '模型观念']
                    },
                    {
                        id: 'j-function', name: '函数', badge: '八—九年级', btype: 'grade',
                        role: '初中代数的高峰，第一次系统接触「变化与对应」的观点。',
                        core: [
                            '函数初步：常量与变量、函数概念、表达式、图象',
                            '一次函数：图象与性质，与方程（组）/不等式的联系',
                            '反比例函数：双曲线图象与性质、k 的几何意义',
                            '二次函数：三种形式、图象与性质、与一元二次方程的关系、最值应用'
                        ],
                        keys: ['自变量与因变量', '定义域', '图象法', '顶点', '对称轴', '增减性'],
                        ideas: ['函数观点', '数形结合']
                    }
                ]
            },
            {
                id: 'j-geometry', name: '图形与几何', badge: '领域二', btype: 'grade',
                role: '暗线是「从直观到推理」——从实验几何过渡到演绎几何。',
                kids: [
                    {
                        id: 'j-geo-prop', name: '图形性质', badge: '七—九年级', btype: 'grade',
                        role: '初中几何的主体，完成「直观感知 → 合情推理 → 演绎推理」三级跳。',
                        core: [
                            '几何图形初步：点线面体、角、相交线与平行线',
                            '三角形：内角和、三边关系；全等三角形判定（SSS/SAS/ASA/AAS/HL）',
                            '轴对称与等腰三角形：垂直平分线、角平分线',
                            '勾股定理及逆定理',
                            '平行四边形及特殊平行四边形（矩形、菱形、正方形）',
                            '圆：性质、圆心角与圆周角、位置关系、弧长与扇形面积',
                            '相似：相似三角形判定与性质、位似、应用'
                        ],
                        keys: ['命题与证明', '辅助线', '全等变换', '勾股数', '圆周角定理', '相似比', '锐角三角函数'],
                        ideas: ['演绎推理', '几何直观']
                    },
                    {
                        id: 'j-geo-trans', name: '图形变化', badge: '七—九年级', btype: 'grade',
                        role: '从「静态性质」扩展到「运动与变换」，为高中解析几何埋下伏笔。',
                        core: [
                            '图形的平移、旋转、对称运动',
                            '轴对称与中心对称',
                            '投影：平行投影、中心投影',
                            '视图：直棱柱、圆柱、圆锥、球的三视图与展开图'
                        ],
                        keys: ['全等变换', '对称性', '三视图'],
                        ideas: ['变换思想', '空间观念']
                    },
                    {
                        id: 'j-geo-coord', name: '图形与坐标', badge: '七—九年级', btype: 'grade',
                        role: '数形结合的第一次正式握手——把几何放进坐标系。',
                        core: [
                            '平面直角坐标系：点的坐标、象限、坐标与距离',
                            '坐标与图形位置：用坐标表示地理位置',
                            '坐标与图形运动：用坐标描述平移、对称、位似'
                        ],
                        keys: ['坐标平面', '坐标与变换'],
                        ideas: ['数形结合思想']
                    },
                    {
                        id: 'j-geo-ruler', name: '尺规作图', badge: '七—八年级', btype: 'grade',
                        role: '从「画图」上升到「用规则构造图形」，体会几何的严谨性。',
                        core: [
                            '作一条线段等于已知线段',
                            '作一个角等于已知角',
                            '作已知角的平分线',
                            '作已知线段的垂直平分线',
                            '过一点作已知直线的垂线'
                        ],
                        keys: ['基本作图', '作图依据']
                    }
                ]
            },
            {
                id: 'j-stat', name: '统计与概率', badge: '领域三', btype: 'grade',
                role: '暗线是「从确定性到随机性」——学会用数据说话、用概率度量不确定。',
                kids: [
                    {
                        id: 'j-stat-collect', name: '数据的收集、整理与描述', badge: '七—八年级', btype: 'grade',
                        role: '统计的第一步：把杂乱的数据变成可读的信息。',
                        core: [
                            '全面调查与抽样调查、总体/个体/样本、简单随机抽样',
                            '统计图表：条形图、扇形图、折线图、频数分布直方图'
                        ],
                        keys: ['样本', '抽样方法', '频数分布'],
                        ideas: ['数据观念']
                    },
                    {
                        id: 'j-stat-analyze', name: '数据的分析', badge: '八年级', btype: 'grade',
                        role: '用数字特征概括数据，并警惕图表的误导。',
                        core: [
                            '集中趋势：平均数、加权平均数、中位数、众数',
                            '离散程度：方差、标准差',
                            '用样本估计总体、统计图表的误用'
                        ],
                        keys: ['平均数/中位数/众数', '方差', '总体估计'],
                        ideas: ['数据观念', '批判性思维']
                    },
                    {
                        id: 'j-prob', name: '概率初步', badge: '九年级', btype: 'grade',
                        role: '第一次量化「可能性」，从频率的稳定性走向概率。',
                        core: [
                            '必然事件、不可能事件、随机事件',
                            '概率的意义与古典概型',
                            '用频率估计概率',
                            '列举法（树状图、列表）计算简单随机事件的概率'
                        ],
                        keys: ['频率稳定性', '古典概型', '列举法'],
                        ideas: ['随机观念']
                    }
                ]
            }
        ]
    },
    senior: {
        tab: '高中 · 必修 + 选择性必修',
        heading: '四大主题 + 建模探究 · 21 个分支',
        groups: [
            {
                id: 's-prep', name: '预备与工具', badge: '基础', btype: 'book',
                role: '高中数学的语言与工具箱，为各主线铺路。',
                kids: [
                    {
                        id: 's-set', name: '集合与常用逻辑用语', badge: '必修一', btype: 'book',
                        role: '数学表达的通用语言，高中第一课。',
                        core: [
                            '集合的含义与表示（列举法、描述法）',
                            '集合间基本关系（子集、真子集、相等）与运算（交、并、补）',
                            '充分条件与必要条件',
                            '全称量词与存在量词、命题的否定'
                        ],
                        keys: ['元素', '集合运算', 'Venn 图', '充要条件', '量词否定'],
                        ideas: ['抽象概括']
                    },
                    {
                        id: 's-ineq', name: '等式与不等式', badge: '必修一', btype: 'book',
                        role: '初中方程的不等式化延伸，「三个二次」在此统一。',
                        core: [
                            '等式性质与不等式性质',
                            '基本不等式（均值不等式）',
                            '一元二次不等式及其解法',
                            '从函数观点看一元二次方程与不等式（三个「二次」的统一）'
                        ],
                        keys: ['均值不等式', '解集', '三个「二次」的关系'],
                        ideas: ['函数观点', '化归思想']
                    },
                    {
                        id: 's-count', name: '计数原理', badge: '选必三', btype: 'book',
                        role: '为概率统计提供计数工具，也是组合思维的入门。',
                        core: [
                            '分类加法计数原理与分步乘法计数原理',
                            '排列与排列数、组合与组合数',
                            '二项式定理及二项式系数的性质'
                        ],
                        keys: ['两个计数原理', '排列数/组合数', '杨辉三角', '二项展开'],
                        ideas: ['分类讨论', '化归思想']
                    }
                ]
            },
            {
                id: 's-function', name: '函数（高中主线）', badge: '主线', btype: 'book',
                role: '贯穿高中三年的纲领，初中函数观点的全面升级。',
                kids: [
                    {
                        id: 's-func-concept', name: '函数概念与性质', badge: '必修一', btype: 'book',
                        role: '用集合与对应重新定义函数，建立研究函数的基本框架。',
                        core: [
                            '函数的概念（定义域、对应关系、值域）',
                            '函数的表示法（解析法、图象法、列表法）',
                            '单调性、奇偶性、最大值最小值',
                            '函数的零点；幂函数'
                        ],
                        keys: ['单调性', '奇偶性', '零点', '幂函数'],
                        ideas: ['函数观点', '数形结合']
                    },
                    {
                        id: 's-exp-log', name: '指数函数与对数函数', badge: '必修一', btype: 'book',
                        role: '第一对互为反函数的基本初等函数，刻画增长与衰减。',
                        core: [
                            '实数指数幂及其运算；指数函数的图象与性质',
                            '对数与对数运算；对数函数',
                            '不同函数增长的差异（直线上升、指数爆炸、对数增长）'
                        ],
                        keys: ['指数/对数运算', '指数爆炸', '反函数'],
                        ideas: ['模型观念']
                    },
                    {
                        id: 's-trig', name: '三角函数', badge: '必修一', btype: 'book',
                        role: '刻画周期现象的基本模型，公式体系最为庞大。',
                        core: [
                            '角的概念推广（弧度制）',
                            '正弦、余弦、正切；同角基本关系、诱导公式',
                            '三角函数的图象与性质',
                            '函数 y = A sin(ωx + φ) 的图象（平移、伸缩变换）'
                        ],
                        keys: ['弧度制', '诱导公式', '周期性', '图象变换'],
                        ideas: ['数形结合', '变换思想']
                    },
                    {
                        id: 's-func-app', name: '函数应用', badge: '必修一', btype: 'book',
                        role: '函数观点的第一次实战：解方程与建模型。',
                        core: [
                            '二分法求方程近似解',
                            '函数模型（指数增长、对数增长）的初步应用'
                        ],
                        keys: ['零点存在性', '二分法', '函数模型'],
                        ideas: ['模型观念', '近似思想']
                    },
                    {
                        id: 's-seq', name: '数列', badge: '选必二', btype: 'book',
                        role: '离散化的函数，递推思想的重要载体。',
                        core: [
                            '数列的概念与表示',
                            '等差数列与等比数列的通项公式与前 n 项和',
                            '递推公式；数学归纳法初步'
                        ],
                        keys: ['通项公式', '前 n 项和', '递推', '数学归纳法'],
                        ideas: ['归纳推理', '递推思想']
                    },
                    {
                        id: 's-deriv', name: '导数及其应用', badge: '选必二', btype: 'book',
                        role: '研究函数的「显微镜」，高中数学的最高峰之一。',
                        core: [
                            '平均变化率与瞬时变化率、导数的概念与几何意义',
                            '导数运算（基本公式、四则运算、复合函数求导）',
                            '用导数研究单调性、极值、最值',
                            '实际问题中的优化'
                        ],
                        keys: ['导数的几何意义', '单调性', '极值与最值'],
                        ideas: ['极限思想', '以直代曲']
                    }
                ]
            },
            {
                id: 's-geo', name: '几何与代数', badge: '两翼', btype: 'book',
                role: '向量与坐标方法取代纯综合法，几何问题全面代数化。',
                kids: [
                    {
                        id: 's-vector', name: '平面向量及其应用', badge: '必修二', btype: 'book',
                        role: '沟通代数与几何的桥梁，解三角形的新工具。',
                        core: [
                            '向量的概念与线性运算、共线定理',
                            '数量积与投影；平面向量基本定理与坐标表示',
                            '正弦定理与余弦定理；解三角形的应用'
                        ],
                        keys: ['自由向量', '线性运算', '数量积', '基本定理', '正余弦定理'],
                        ideas: ['向量方法', '数形结合']
                    },
                    {
                        id: 's-complex', name: '复数', badge: '必修二', btype: 'book',
                        role: '数系的第二次扩张：实数 → 复数。',
                        core: [
                            '数系的扩充与复数的概念；几何表示',
                            '复数的四则运算',
                            '复数的三角形式、乘除运算的几何意义'
                        ],
                        keys: ['虚数单位 i', '复平面', '模与辐角'],
                        ideas: ['数系扩张思想']
                    },
                    {
                        id: 's-solid', name: '立体几何初步', badge: '必修二', btype: 'book',
                        role: '从平面走向空间，空间想象力的系统训练。',
                        core: [
                            '空间几何体的结构特征（柱、锥、球）；三视图与直观图',
                            '表面积与体积',
                            '空间点、直线、平面的位置关系',
                            '直线、平面平行与垂直的判定与性质'
                        ],
                        keys: ['线面关系', '平行与垂直', '表面积/体积'],
                        ideas: ['空间观念', '演绎推理']
                    },
                    {
                        id: 's-space-vec', name: '空间向量与立体几何', badge: '选必一', btype: 'book',
                        role: '立体几何的「坐标化」——把位置关系变成代数运算。',
                        core: [
                            '空间直角坐标系；空间向量及其运算',
                            '用空间向量处理平行、垂直、角度、距离'
                        ],
                        keys: ['空间向量', '法向量', '向量法求角'],
                        ideas: ['向量方法', '代数化']
                    },
                    {
                        id: 's-analytic', name: '解析几何', badge: '选必一', btype: 'book',
                        role: '数形结合的最高形态：曲线即方程，方程即曲线。',
                        kids: [
                            {
                                id: 's-line-circle', name: '直线与圆', badge: '选必一', btype: 'book',
                                role: '解析几何的入门：最简单的曲线，最基本的方法。',
                                core: [
                                    '直线的倾斜角与斜率、直线方程（点斜式、斜截式、一般式）',
                                    '两条直线的位置关系、点到直线的距离',
                                    '圆的方程、直线与圆的位置关系'
                                ],
                                keys: ['斜率', '直线方程', '距离公式'],
                                ideas: ['坐标方法']
                            },
                            {
                                id: 's-conic', name: '圆锥曲线', badge: '选必一', btype: 'book',
                                role: '高中几何的压轴内容，定义、方程与性质的完整闭环。',
                                core: [
                                    '椭圆、双曲线、抛物线的定义',
                                    '标准方程与几何性质',
                                    '曲线与方程的思想'
                                ],
                                keys: ['标准方程', '离心率', '统一定义（第二定义）', '焦点弦'],
                                ideas: ['曲线与方程', '数形结合']
                            }
                        ]
                    }
                ]
            },
            {
                id: 's-stat', name: '统计与概率', badge: '应用', btype: 'book',
                role: '从描述数据到推断总体，随机观念的完整建立。',
                kids: [
                    {
                        id: 's-stat-basic', name: '统计', badge: '必修二', btype: 'book',
                        role: '初中数据分析的升级版：更严谨的抽样与估计。',
                        core: [
                            '随机抽样（简单随机抽样、分层抽样）',
                            '用样本估计总体（频率分布直方图、样本数字特征）',
                            '统计图表在生活中的应用'
                        ],
                        keys: ['分层抽样', '频率分布', '百分位数'],
                        ideas: ['数据观念']
                    },
                    {
                        id: 's-prob-basic', name: '概率', badge: '必修二', btype: 'book',
                        role: '概率的公理化初步：从古典概型走向一般事件运算。',
                        core: [
                            '有限样本空间与随机事件',
                            '事件的关系与运算（和、积、互斥、对立）',
                            '古典概型、几何概型',
                            '事件的独立性、条件概率'
                        ],
                        keys: ['样本空间', '互斥/独立', '条件概率'],
                        ideas: ['随机观念']
                    },
                    {
                        id: 's-random-var', name: '随机变量及其分布', badge: '选必三', btype: 'book',
                        role: '用函数观点研究随机现象，概率论的现代入口。',
                        core: [
                            '离散型随机变量的分布列、均值与方差',
                            '二项分布与超几何分布',
                            '正态分布；随机变量的独立性'
                        ],
                        keys: ['分布列', '期望与方差', '二项分布', '正态分布'],
                        ideas: ['函数观点', '模型观念']
                    },
                    {
                        id: 's-regression', name: '成对数据的统计分析', badge: '选必三', btype: 'book',
                        role: '统计推断的实战：变量之间的关系有多可靠？',
                        core: [
                            '一元线性回归模型',
                            '2×2 列联表与独立性检验'
                        ],
                        keys: ['回归分析', '相关系数', '独立性检验'],
                        ideas: ['统计推断']
                    }
                ]
            },
            {
                id: 's-model', name: '数学建模与探究', badge: '实践', btype: 'book',
                role: '不是独立知识点，而是贯穿全程的综合实践主题。',
                kids: [
                    {
                        id: 's-modeling', name: '数学建模活动', badge: '必修/选必', btype: 'book',
                        role: '训练「用数学解决真实问题」的完整流程。',
                        core: [
                            '选题、提出假设、建立模型、求解模型、检验与修正',
                            '典型项目：行程优化、体重与脉搏、包装成本测算'
                        ],
                        keys: ['建模流程', '假设检验', '迭代修正'],
                        ideas: ['模型观念', '应用意识']
                    },
                    {
                        id: 's-inquiry', name: '数学探究活动', badge: '必修/选必', btype: 'book',
                        role: '从特殊到一般的猜想、归纳与证明。',
                        core: [
                            '观察特例、提出猜想、严格证明',
                            '经典素材：斐波那契数列等'
                        ],
                        keys: ['归纳猜想', '证明'],
                        ideas: ['归纳推理', '创新意识']
                    }
                ]
            }
        ]
    }
};

// 总览页的静态内容（衔接对照表等与 syllabus.md 第三部分一致）
var OVERVIEW = {
    pillars: [
        { name: '函数主线', desc: '概念性质 → 指数对数 → 三角 → 数列 → 导数，贯穿高中三年。' },
        { name: '几何与代数', desc: '向量 → 复数 → 立体几何 → 空间向量 → 解析几何，几何代数化。' },
        { name: '统计与概率', desc: '统计 → 概率 → 随机变量 → 回归分析，从描述走向推断。' },
        { name: '预备与工具', desc: '集合逻辑、不等式、计数原理——支撑各主线的语言与工具。' }
    ],
    courses: [
        { name: '必修 第一册', kind: 'must', desc: '预备知识（集合与逻辑、不等式）· 函数' },
        { name: '必修 第二册', kind: 'must', desc: '几何与代数（向量、复数、立体几何初步）· 统计与概率' },
        { name: '选择性必修 第一册', kind: 'elective', desc: '空间向量 · 解析几何（直线与圆、圆锥曲线）' },
        { name: '选择性必修 第二册', kind: 'elective', desc: '数列 · 导数及其应用' },
        { name: '选择性必修 第三册', kind: 'elective', desc: '计数原理 · 随机变量 · 统计案例' }
    ],
    links: [
        ['实数与代数式', '复数、不等式', '数系的两次扩张：实数 → 复数'],
        ['一元二次方程', '一元二次不等式、二次函数', '「三个二次」在函数观点下统一'],
        ['一次/二次/反比例函数', '三角函数、指数对数函数、幂函数', '函数观点成为高中数学纲领'],
        ['平面几何（全等、相似）', '立体几何、解析几何、向量几何', '综合几何 → 向量/坐标方法'],
        ['三角形与解直角三角形', '正弦定理余弦定理、三角函数', '从直角走向一般三角形'],
        ['数据的分析、古典概型', '随机变量及其分布、统计推断', '从描述数据到推断总体'],
        ['—（新增）', '导数、数列、计数原理', '高中的三块新增工具']
    ]
};

// ---------- 模块级状态 ----------

var state = {
    root: null,          // 页面根容器（#mathApp）
    stage: 'overview',   // 当前标签页：overview | junior | senior
    selectedId: null,    // 当前选中的分支节点 id
    collapsed: {},       // id → true 表示折叠（默认全部展开）
    query: '',           // 搜索关键词（小写）
    nodeIndex: null      // id → { node, parent, path[] }，init 时一次性建好
};

// ---------- 工具 ----------

function el(tag, cls, text) {
    var node = document.createElement(tag);
    if (cls) node.className = cls;
    if (text != null) node.textContent = text;
    return node;
}

// 依据模块自身地址注入 math.css，避免主站注入时相对路径解析错误
function ensureCss() {
    if (document.getElementById('math-module-css')) return;
    var link = document.createElement('link');
    link.id = 'math-module-css';
    link.rel = 'stylesheet';
    link.href = new URL('./math.css', import.meta.url).href;
    document.head.appendChild(link);
}

// 建索引：id → { node, parent, path }；path 是从根到节点的名称链（面包屑用）
function buildIndex() {
    var index = {};
    function walk(node, parent, path) {
        var myPath = path.concat(node.name);
        index[node.id] = { node: node, parent: parent, path: myPath };
        if (node.kids) {
            for (var i = 0; i < node.kids.length; i++) walk(node.kids[i], node, myPath);
        }
    }
    ['junior', 'senior'].forEach(function (stage) {
        var stageName = stage === 'junior' ? '初中' : '高中';
        DATA[stage].groups.forEach(function (g) { walk(g, null, [stageName]); });
    });
    return index;
}

function countLeaves(groups) {
    var n = 0;
    groups.forEach(function (g) {
        g.kids.forEach(function (k) { n += k.kids ? k.kids.length : 1; });
    });
    return n;
}

// 命中判断：名称 / 定位 / 核心内容 / 关键概念任一包含关键词
function nodeMatches(node, q) {
    if (!q) return true;
    var hay = node.name + ' ' + (node.role || '') + ' ' +
        (node.core || []).join(' ') + ' ' + (node.keys || []).join(' ') + ' ' +
        (node.ideas || []).join(' ');
    return hay.toLowerCase().indexOf(q) !== -1;
}

// ---------- 渲染：头部 ----------

function renderHero() {
    var box = state.root.querySelector('#mathHeroStats');
    box.innerHTML = '';
    var juniorLeaves = countLeaves(DATA.junior.groups);
    var seniorLeaves = countLeaves(DATA.senior.groups);
    var stats = [
        { num: 2, label: '学段' },
        { num: DATA.junior.groups.length + DATA.senior.groups.length, label: '领域 / 主题' },
        { num: juniorLeaves + seniorLeaves, label: '细分分支' }
    ];
    stats.forEach(function (s, i) {
        var chip = el('span', 'math-stat-chip math-anim-item');
        chip.style.animationDelay = (i * 90) + 'ms';
        var b = el('b', null, '0');
        chip.appendChild(b);
        chip.appendChild(document.createTextNode(' ' + s.label));
        box.appendChild(chip);
        // 260ms 起步：页面本身有 220ms 淡入，数字等淡入结束再滚，
        // 否则用户会先看到一排「0」（原来 600ms 起，切换学科时明显发愣）
        countUp(b, s.num, 260 + i * 70);
    });
}

// 数字滚动：从 0 增到 target，时长 ~500ms（帧率自适应）
function countUp(node, target, delay) {
    // 无障碍：系统开启「减少动态效果」时直接落到终值，不做滚动
    if (window.matchMedia && window.matchMedia('(prefers-reduced-motion: reduce)').matches) {
        node.textContent = String(target);
        return;
    }
    var start = null;
    var DURATION = 500;
    function tick(ts) {
        if (start === null) start = ts;
        var p = Math.min(1, (ts - start) / DURATION);
        // easeOutCubic，和整体动画的缓动气质一致
        node.textContent = String(Math.round(target * (1 - Math.pow(1 - p, 3))));
        if (p < 1) requestAnimationFrame(tick);
    }
    setTimeout(function () { requestAnimationFrame(tick); }, delay || 0);
}

// ---------- 渲染：标签页 ----------

var TABS = [
    { id: 'overview', label: '总览' },
    { id: 'junior', label: '初中' },
    { id: 'senior', label: '高中' }
];

function renderTabs() {
    var nav = state.root.querySelector('#mathTabs');
    nav.innerHTML = '';
    TABS.forEach(function (t) {
        var btn = el('button', 'math-tab' + (state.stage === t.id ? ' is-active' : ''), t.label);
        btn.setAttribute('role', 'tab');
        btn.setAttribute('aria-selected', state.stage === t.id ? 'true' : 'false');
        btn.addEventListener('click', function () { switchStage(t.id); });
        nav.appendChild(btn);
    });
}

// 切换标签页：先淡出（is-out）再替换内容并淡入（is-in），节拍与 english 的 studyOut/studyIn 一致
function switchStage(id) {
    if (id === state.stage) return;
    var body = state.root.querySelector('#mathBody');
    body.classList.remove('is-in');
    body.classList.add('is-out');
    setTimeout(function () {
        state.stage = id;
        state.selectedId = null;
        renderTabs();
        renderBody();
        body.classList.remove('is-out');
        body.classList.add('is-in');
    }, 160);
}

// ---------- 渲染：主体 ----------

function renderBody() {
    var toolbar = state.root.querySelector('#mathToolbar');
    var body = state.root.querySelector('#mathBody');
    body.innerHTML = '';
    if (state.stage === 'overview') {
        toolbar.hidden = true;
        body.classList.remove('is-tree-mode');
        renderOverview(body);
    } else {
        toolbar.hidden = false;
        body.classList.add('is-tree-mode');
        renderTreeMode(body, DATA[state.stage]);
    }
}

// ---------- 总览视图 ----------

function renderOverview(body) {
    var wrap = el('div', 'math-overview');
    body.appendChild(wrap);

    // 学段流向卡
    var flowCard = el('section', 'math-ov-card math-anim-item');
    flowCard.appendChild(el('h3', null, '学段衔接总览'));
    var flow = el('div', 'math-ov-flow');
    var n1 = el('div', 'math-ov-flow-node is-stage');
    n1.appendChild(el('b', null, '初中'));
    n1.appendChild(document.createTextNode('三大领域平行推进：数与代数 · 图形与几何 · 统计与概率'));
    var n2 = el('div', 'math-ov-flow-node is-high');
    n2.appendChild(el('b', null, '高中'));
    n2.appendChild(document.createTextNode('统合为四大主题：函数主线 · 几何与代数 · 统计与概率 · 预备与工具'));
    var n3 = el('div', 'math-ov-flow-node is-result');
    n3.appendChild(el('b', null, '能力指向'));
    n3.appendChild(document.createTextNode('抽象概括 · 逻辑推理 · 数学建模 · 直观想象 · 数学运算 · 数据分析'));
    flow.appendChild(n1);
    flow.appendChild(el('span', 'math-ov-arrow', '→'));
    flow.appendChild(n2);
    flow.appendChild(el('span', 'math-ov-arrow', '→'));
    flow.appendChild(n3);
    flowCard.appendChild(flow);
    wrap.appendChild(flowCard);

    // 高中四大主题
    var pillarCard = el('section', 'math-ov-card math-anim-item');
    pillarCard.style.animationDelay = '80ms';
    pillarCard.appendChild(el('h3', null, '高中四大主题'));
    var pillars = el('div', 'math-ov-pillars');
    OVERVIEW.pillars.forEach(function (p) {
        var d = el('div', 'math-ov-pillar');
        d.appendChild(el('b', null, p.name));
        d.appendChild(document.createTextNode(p.desc));
        pillars.appendChild(d);
    });
    pillarCard.appendChild(pillars);
    wrap.appendChild(pillarCard);

    // 课程结构
    var courseCard = el('section', 'math-ov-card math-anim-item');
    courseCard.style.animationDelay = '160ms';
    courseCard.appendChild(el('h3', null, '高中课程结构'));
    var course = el('div', 'math-ov-course');
    OVERVIEW.courses.forEach(function (c) {
        var d = el('div', 'math-ov-course-card is-' + c.kind);
        d.appendChild(el('b', null, c.name));
        d.appendChild(document.createTextNode(c.desc));
        course.appendChild(d);
    });
    courseCard.appendChild(course);
    wrap.appendChild(courseCard);

    // 初高中衔接对照表
    var linkCard = el('section', 'math-ov-card math-anim-item');
    linkCard.style.animationDelay = '240ms';
    linkCard.appendChild(el('h3', null, '初高中分支衔接对照'));
    var table = el('table', 'math-ov-table');
    var thead = el('thead');
    var hr = el('tr');
    ['初中分支', '在高中延展为', '核心线索'].forEach(function (h) { hr.appendChild(el('th', null, h)); });
    thead.appendChild(hr);
    table.appendChild(thead);
    var tbody = el('tbody');
    OVERVIEW.links.forEach(function (row) {
        var tr = el('tr');
        row.forEach(function (cell) { tr.appendChild(el('td', null, cell)); });
        tbody.appendChild(tr);
    });
    table.appendChild(tbody);
    linkCard.appendChild(table);
    wrap.appendChild(linkCard);
}

// ---------- 树 + 详情视图 ----------

function renderTreeMode(body, stageData) {
    var treePanel = el('aside', 'math-tree-panel');
    treePanel.appendChild(el('h2', 'math-tree-heading', stageData.heading));
    var tree = el('div', 'math-tree');
    treePanel.appendChild(tree);
    body.appendChild(treePanel);

    var detail = el('section', 'math-detail-panel', null);
    detail.id = 'mathDetail';
    body.appendChild(detail);

    renderTree(tree, stageData.groups);
    renderDetail();
}

function renderTree(container, groups) {
    container.innerHTML = '';
    var ul = el('ul');
    groups.forEach(function (g) { ul.appendChild(renderNode(g, 0)); });
    container.appendChild(ul);
    applySearchVisibility(container);
}

function renderNode(node, depth) {
    var li = el('li');
    var row = el('button', 'math-node-row', null);
    row.type = 'button';
    row.dataset.nodeId = node.id;

    var twisty = el('span', 'math-twisty' + (node.kids ? '' : ' is-leaf'), '▶');
    row.appendChild(twisty);
    row.appendChild(el('span', 'math-node-name', node.name));
    if (node.badge) {
        row.appendChild(el('span', 'math-node-badge is-' + (node.btype || 'grade'), node.badge));
    }

    row.addEventListener('click', function () {
        if (node.kids && state.collapsed[node.id] === undefined && state.selectedId === node.id) {
            // 已选中且未折叠：再点一次 = 折叠
            toggleCollapse(node.id, true);
        } else if (node.kids && state.collapsed[node.id]) {
            toggleCollapse(node.id, false);
            selectNode(node.id);
        } else {
            selectNode(node.id);
        }
    });
    li.appendChild(row);

    if (node.kids) {
        var sub = el('ul');
        node.kids.forEach(function (k) { sub.appendChild(renderNode(k, depth + 1)); });
        if (state.collapsed[node.id]) sub.hidden = true;
        li.appendChild(sub);
        updateTwisty(node.id, li);
    }
    return li;
}

function updateTwisty(id, li) {
    var row = (li || findRowLi(id)).querySelector('.math-twisty');
    if (row) row.classList.toggle('is-open', !state.collapsed[id]);
}

function findRowLi(id) {
    var row = state.root.querySelector('.math-node-row[data-node-id="' + id + '"]');
    return row ? row.parentNode : null;
}

// 折叠/展开：子列表用高度动画（grid-rows 技巧），节点出现带轻微交错
function toggleCollapse(id, collapse) {
    state.collapsed[id] = collapse ? true : undefined;
    if (!collapse) delete state.collapsed[id];
    var li = findRowLi(id);
    if (!li) return;
    var sub = li.querySelector('ul');
    if (!sub) return;
    updateTwisty(id, li);
    if (collapse) {
        sub.classList.add('math-collapsing');
        setTimeout(function () {
            sub.hidden = true;
            sub.classList.remove('math-collapsing');
        }, 180);
    } else {
        sub.hidden = false;
        // 交错入场：每行延迟 40ms，最多 6 行
        var rows = sub.querySelectorAll(':scope > li > .math-node-row');
        for (var i = 0; i < rows.length; i++) {
            rows[i].classList.add('math-row-in');
            rows[i].style.animationDelay = Math.min(i, 6) * 40 + 'ms';
        }
        setTimeout(function () {
            for (var j = 0; j < rows.length; j++) {
                rows[j].classList.remove('math-row-in');
                rows[j].style.animationDelay = '';
            }
        }, 40 * Math.min(rows.length, 6) + 320);
    }
}

function selectNode(id) {
    state.selectedId = id;
    var prev = state.root.querySelector('.math-node-row.is-selected');
    if (prev) prev.classList.remove('is-selected');
    var li = findRowLi(id);
    if (li) li.querySelector('.math-node-row').classList.add('is-selected');
    renderDetail();
}

// ---------- 详情面板 ----------

function renderDetail() {
    var panel = state.root.querySelector('#mathDetail');
    if (!panel) return;
    panel.innerHTML = '';
    var entry = state.selectedId && state.nodeIndex[state.selectedId];
    if (!entry) {
        panel.appendChild(el('div', 'math-detail-placeholder', '点击左侧分支，查看定位、核心内容与关键概念'));
        return;
    }
    var node = entry.node;

    panel.appendChild(el('div', 'math-detail-crumb', entry.path.join(' / ')));
    panel.appendChild(el('h3', 'math-detail-title', node.name));
    if (node.role) panel.appendChild(el('p', 'math-detail-role', node.role));

    if (node.core && node.core.length) {
        var sec = el('div', 'math-detail-section math-detail-anim');
        sec.appendChild(el('h4', null, '核心内容'));
        var ul = el('ul');
        node.core.forEach(function (c, i) {
            var li = el('li', 'math-anim-item', c);
            li.style.animationDelay = (i * 50) + 'ms';
            ul.appendChild(li);
        });
        sec.appendChild(ul);
        panel.appendChild(sec);
    }
    if (node.keys && node.keys.length) {
        var sec2 = el('div', 'math-detail-section math-detail-anim');
        sec2.style.animationDelay = '90ms';
        sec2.appendChild(el('h4', null, '关键概念'));
        sec2.appendChild(chipRow(node.keys, false));
        panel.appendChild(sec2);
    }
    if (node.ideas && node.ideas.length) {
        var sec3 = el('div', 'math-detail-section math-detail-anim');
        sec3.style.animationDelay = '160ms';
        sec3.appendChild(el('h4', null, '思想方法'));
        sec3.appendChild(chipRow(node.ideas, true));
        panel.appendChild(sec3);
    }
    if (node.kids && node.kids.length) {
        var sec4 = el('div', 'math-detail-section math-detail-anim');
        sec4.style.animationDelay = '220ms';
        sec4.appendChild(el('h4', null, '下设分支'));
        var ul2 = el('ul');
        node.kids.forEach(function (k, i) {
            var li = el('li', 'math-anim-item', k.name + (k.role ? '——' + k.role : ''));
            li.style.animationDelay = (i * 50) + 'ms';
            ul2.appendChild(li);
        });
        sec4.appendChild(ul2);
        panel.appendChild(sec4);
    }
}

function chipRow(words, isIdea) {
    var box = el('div', 'math-chips');
    words.forEach(function (w, i) {
        var c = el('span', 'math-chip math-anim-item' + (isIdea ? ' is-idea' : ''), w);
        c.style.animationDelay = (i * 45) + 'ms';
        box.appendChild(c);
    });
    return box;
}

// ---------- 搜索 ----------

function onSearchInput(e) {
    state.query = e.target.value.trim().toLowerCase();
    var container = state.root.querySelector('.math-tree');
    if (container) applySearchVisibility(container);
}

// 搜索策略：不删节点（保持树形完整），只淡化未命中的行；
// 有命中时自动展开所有折叠节点，让命中处可见。
function applySearchVisibility(container) {
    var q = state.query;
    var rows = container.querySelectorAll('.math-node-row');
    for (var i = 0; i < rows.length; i++) {
        var id = rows[i].dataset.nodeId;
        var entry = state.nodeIndex[id];
        if (!entry) continue;
        var hit = !q || subtreeMatches(entry.node, q);
        rows[i].classList.toggle('is-dimmed', !hit);
    }
    if (q) {
        // 展开全部，保证命中行不被折叠藏住
        var subs = container.querySelectorAll('ul ul');
        for (var j = 0; j < subs.length; j++) subs[j].hidden = false;
        var twisties = container.querySelectorAll('.math-twisty');
        for (var k = 0; k < twisties.length; k++) twisties[k].classList.add('is-open');
    }
}

// 自身或任一后代命中即算命中（父节点命中时保留整个子树）
function subtreeMatches(node, q) {
    if (nodeMatches(node, q)) return true;
    if (!node.kids) return false;
    for (var i = 0; i < node.kids.length; i++) {
        if (subtreeMatches(node.kids[i], q)) return true;
    }
    return false;
}

// ---------- 展开/收起全部 ----------

function toggleExpandAll(btn) {
    var anyCollapsed = Object.keys(state.collapsed).length > 0;
    var groups = DATA[state.stage].groups;
    if (anyCollapsed) {
        state.collapsed = {};
    } else {
        groups.forEach(function (g) {
            state.collapsed[g.id] = true;
            (g.kids || []).forEach(function (k) { if (k.kids) state.collapsed[k.id] = true; });
        });
    }
    btn.textContent = anyCollapsed ? '全部收起' : '全部展开';
    var container = state.root.querySelector('.math-tree');
    if (container) renderTree(container, groups);
}

// ---------- 入口 ----------

function boot(container) {
    state.root = container;
    state.nodeIndex = buildIndex();
    ensureCss();
    renderHero();
    renderTabs();
    renderBody();
    var body = state.root.querySelector('#mathBody');
    body.classList.add('is-in');

    var search = state.root.querySelector('#mathSearch');
    search.addEventListener('input', onSearchInput);
    var expandBtn = state.root.querySelector('#mathExpandBtn');
    expandBtn.addEventListener('click', function () { toggleExpandAll(expandBtn); });
}

// 主站接入时调用：container 为注入 math.html 后的根元素
export function initMathApp(container) {
    boot(container || document.getElementById('mathApp'));
}

export function unmountMath() {
    state.root = null;
    state.selectedId = null;
    state.collapsed = {};
    state.query = '';
}

// main.js 的 teardownSubjectModule() 按约定只认 unmount()，这里给它一个同义导出，
// 免得切学科时上一个数学页的状态（选中项 / 折叠表 / 搜索词）留在内存里。
export function unmount() {
    unmountMath();
    // 样式表随模块一起撤掉：留在 <head> 里会一直参与全站样式计算，
    // 而切回数学页时 ensureCss() 会重新注入。
    var link = document.getElementById('math-module-css');
    if (link) link.remove();
}

// 本地直接打开 math.html 时自举（页面里已置 window.__MATH_STANDALONE）
if (typeof window !== 'undefined' && window.__MATH_STANDALONE) {
    if (document.readyState === 'loading') {
        document.addEventListener('DOMContentLoaded', function () { boot(document.getElementById('mathApp')); });
    } else {
        boot(document.getElementById('mathApp'));
    }
}
