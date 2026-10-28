export const c3Tasks = [
  {
    "id": 1,
    "title": "包契约与预算",
    "status": "done",
    "detail": "版本化 manifest、精确对象身份与摘要、阅读副本隔离、封闭预算。",
    "evidence": "代码与独立复审完成；最终纳入 C3 整关验收。"
  },
  {
    "id": 2,
    "title": "一致快照求闭包",
    "status": "done",
    "detail": "在单个只读事务中固定 Reading、引用、关系、审查和资产使用，并按当前授权过滤。",
    "evidence": "全新隔离 PostgreSQL：专项 9/9、相关回归 15/15。"
  },
  {
    "id": 3,
    "title": "目录包与阅读副本",
    "status": "done",
    "detail": "私有 staging、流式核哈希、Linux no-follow 与受控句柄交付；阅读副本去身份化。",
    "evidence": "独立复审通过；全新隔离 Linux 专项 38/38。"
  },
  {
    "id": 4,
    "title": "作业、租约与撤权交付",
    "status": "done",
    "detail": "追加 C3 作业类型与迁移；复用租约 fencing，完成和交付时重新核验授权。",
    "evidence": "独立复审通过；全新隔离 PostgreSQL 专项 18/18、C2 回归 39/39、Worker 11 项及 0013→0014 升级通过；修复已并入主工作树。"
  },
  {
    "id": 5,
    "title": "导入预检",
    "status": "done",
    "detail": "校验版本、路径、摘要、身份冲突、引用闭包及目标资产存在性。",
    "evidence": "夹具修复后独立复审通过；全新隔离项目 Task 5 12/12，相关回归合计 86 项、格式/严格 Clippy 通过。"
  },
  {
    "id": 6,
    "title": "单事务精确导入",
    "status": "done",
    "detail": "同 ID 同字节幂等复用，冲突拒绝；结果原子可见且可追溯。",
    "evidence": "第三轮新隔离 PostgreSQL 专项 3/3、指定回归与 fmt/严格 Clippy 通过；旧夹具失败和缺冻结清单的 101 均保留证据。"
  },
  {
    "id": 7,
    "title": "隔离端到端验收",
    "status": "done",
    "detail": "完整往返、故障注入、撤权、旧版升级、全工作区与独立复审。",
    "evidence": "全量 result SHA c939cc71…；602 份证据哈希一致；整关已验收，未生产部署。"
  }
];

export const c4ParentMilestones = [
  {
    "id": 1,
    "title": "全量备份契约与资产清单",
    "status": "done",
    "detail": "版本化 manifest、规范摘要与全部 ready 资产目录；规划不冒充完成备份。",
    "evidence": "独立复审及全新 PG18 专项通过，含普通角色拒绝；C4 整关未验收。"
  },
  {
    "id": 2,
    "title": "安全封存与目标端校验",
    "status": "done",
    "detail": "私有 staging、流式复制核哈希、no-follow 与封存；中断不发布可恢复包。",
    "evidence": "Linux 专项及 SIGKILL/部分写入故障门通过；同步故障使用测试钩子，非真实掉电证明。"
  },
  {
    "id": 3,
    "title": "维护窗与一致性数据库备份",
    "status": "next",
    "detail": "受控写闸、在途排空、一致 dump 与资产清单；由补齐计划Task1–3继续完成。",
    "evidence": "源端与单机保留/传输补验已收尾；真实独立目标、完成收据与发布权限仍归Task9，父里程碑保持开放。"
  },
  {
    "id": 4,
    "title": "干净实例恢复",
    "status": "next",
    "detail": "目标准入、出生证明、精确端点绑定与完整导入；由补齐计划Task4–6继续完成。",
    "evidence": "Task4内容、Task5受控导入、Task6私有16/16及Task8规定回归均有分项证据；正常公开恢复资格仍待Task9，父里程碑保持开放。"
  },
  {
    "id": 5,
    "title": "完整场景与故障注入验收",
    "status": "next",
    "detail": "完整 Attention、当前完整回归、四套旧版升级与整分支审查；由补齐计划Task7–9闭合。",
    "evidence": "Task7私有六案及Task8完整回归/四升级/规定专项/整组审查已限定关闭；Task9独立故障域与正常整链尚未满足，父里程碑保持开放。"
  }
];

export const c4Tasks = [
  {
    "id": 1,
    "title": "持久登记与原件保护",
    "status": "done",
    "statusLabel": "限定分项已验收",
    "detail": "登记资产与本地 pin 根身份；独立发现保留保护，缺失或不完整时拒绝清理候选。",
    "evidence": "固定 Linux/PG 登记保护门 11/11 与五项文件系统门通过；本任务不签发完整备份，也不关闭 C4。"
  },
  {
    "id": 2,
    "title": "精确源端绑定与捕获监督",
    "status": "done",
    "statusLabel": "代码与单机补验收尾完成",
    "detail": "历史ba96561同包准入5/5、绑定4+2、新端点5/5及fresh life8/8已分别独审接受。",
    "evidence": "同包准入5/5、绑定4+2、新端点5/5和fresh life8/8分别独审采用。历史UNKNOWN和失败仍保留；当前源码全量回归归Task8。"
  },
  {
    "id": 3,
    "title": "保留校验与完成收据",
    "status": "done",
    "statusLabel": "单机补验收尾完成，Task9开放",
    "detail": "校验封存包和保留集；真实首签与新发布入口默认关闭，跨操作权限、独立目标和密钥托管归Task9。",
    "evidence": "当前Windows16项与Linux默认20/特性配置6项分别通过；629原件、17执行记录、539源码与8编译角色关联已核对，单机补验收尾。8个相关Git blob同已验版本；历史E8两项Linux目标文件校验/中断传输已核验，属于合成历史分项，未签发真实完成收据。不同源码/平台分别留证，不用可见残留推导CompleteBackup。"
  },
  {
    "id": 4,
    "title": "全量模式与有界 COPY 数据",
    "status": "done",
    "statusLabel": "内容范围3/3通过",
    "detail": "完整审查模式、固定客户端/TOC 与有界 COPY 字节；函数/ACL 篡改拒绝，数据中的 SQL 外观不变成命令。",
    "evidence": "只关闭模式/TOC/COPY内容三门，不授予整库恢复资格；最终当前源码回归归Task8。"
  },
  {
    "id": 5,
    "title": "受控整库导入与原件恢复",
    "status": "done",
    "statusLabel": "代码与规定单机补验限定完成",
    "detail": "受控整库导入、提交前拒绝、取消与未知提交结果、角色/路由/目标/原件检查，按各次固定源码和实际回执分别留证。",
    "evidence": "2026-10-20 Root限定采用：85c九项native及最终Linux回归；受控11/11为10历史c749＋1当前85c，不能称当前源码新跑11项。旧失败与HOLD保留。"
  },
  {
    "id": 6,
    "title": "作业归一化与恢复资格校验",
    "status": "done",
    "statusLabel": "私有单机16/16限定完成",
    "detail": "恢复后全表校验、任务归一化、旧租约/token失效、快照写入与效果证明；无法确认的外部副作用保留Unknown/HOLD。",
    "evidence": "2026-10-24采用2e5026d：五项恢复基础＋十一项快照场景，每项1 passed/0 failed/0 ignored，含core/P6原件独核。该历史范围不等于当前源码Task8回归或公开恢复资格。"
  },
  {
    "id": 7,
    "title": "完整 Attention 链与负例",
    "status": "done",
    "detail": "Attention完整正例、缺失原件、损坏dump、传输中断、未知外部效果、原件根替换六案，按各自预期阶段采用。",
    "evidence": "2026-10-25 Root采用36c89f2；此前3项＋新3项，六案Core/Proof与三个good-pin场景P6及实际ReadingCopy字节独核。仅规定私有单机夹具，不签发CompleteBackup/RecoveredInstance。",
    "statusLabel": "私有单机6/6原件限定关闭"
  },
  {
    "id": 8,
    "title": "当前回归、旧版升级与审查",
    "status": "done",
    "detail": "Task8已按规定私有单机范围完成：A5完整Linux/PG18十门、四套真实旧版升级、54项fresh专项（53项A5＋1项Source679）、6项未变历史桥及整组独审。RootFS33与生命周期FS5单列；各源码与证明层级分别记账。",
    "evidence": "全部规定专项的有限原件、来源、实际正文与精确资源收尾已限定采用；所有旧失败/HOLD保持。生命周期真实13,094日志经独审后的16k读取配方完整导出13,099文件，512MiB读/输出、32MiB单件、300/360秒与资源等式不变。Full9大negative、部分最终proof与Schema未选output、ELF/inspect/独立OS时钟等层级限制继续披露，不宣称所有物理状态或正常恢复均已外部证明。 169 ignored已具名处置，不等于169全部运行；65 extra与3 measurement未执行披露。",
    "statusLabel": "私有单机范围限定验收"
  },
  {
    "id": 9,
    "title": "真实独立故障域闭合",
    "status": "next",
    "statusLabel": "DB3限定普通检查通过，Root／M3接线中",
    "detail": "DB3六个既有源码叶完成限定普通Linux13门：DB默认62通过／24忽略、peer117通过／26忽略，均0失败／0过滤；peer新增12个独立普通正文通过，配置间重复正文不相加。backup默认与offhost只有依赖check。真实Root发行、M3整个future／finally、M5、C11整链、S5封闭管线与Task9／C4验收仍开放。",
    "evidence": "批次2fb62369-09c1-41fa-8126-9d73b12fe40f，result 1dbfec0f2548f14f601b8898ea2e7dd6d35577570f795328c188a909de5a33bc；evidence ZIP 0f6eb27597aeaf456764c47771dfa77323dce89e4113c86fb22b9986b7316c6b。源CI 546f0cded8c37a685e914874610cfbd80aa0f7e02400562804caac8aa2d018a9／MF 922ebbee717c11ba9256a4f617b47c205412fc166bb90bc4d477ed20b41758e1／TEST 5882ad2f4be6eeff7e30c7d9e35f362dbf25d751f4aba7d4a4635966e0531cb5，Root CODE b45411a2b78eff8448a77456c5b37e15e8f17818b27374654c41d39ddd64f651。最终checks CONTROL 0c97cc470c8e51a20bb24812a9fbca6436d13214087df8b509f63e8ab1be39ab；独立完整原件OTHER 296108a050d5e17956f511b342c0f5df9fccd7d2e333ffbfecdcdbbcb117ad6c／INDEX bdd755139f3112a3d9d6501c4d86ecbbef2ab2176841b39eae125cc92a370322。实际13门全exit0、完整86／143名单与24／26顶层ignored、共享历史资源和新12正文，按该独审有限范围核证；源码原件未变、精确CID自然退出及清理、返回证据链已核。不是独立远程亲见、连续OS cap或全链资源计费证明。"
  }
];

export const diagrams = [
  {
    "id": "overview",
    "title": "01 · 全系统架构总览",
    "subtitle": "模块化单体目标架构：业务契约与现有后端内核已实现；HTTP/API和Web产品入口待P1建设。",
    "notes": [
      "浏览器工作台、HTTP 登录和学习闭环均为待建设。React + TypeScript + Vite、Axum 是建议组合，尚未作为已实施依赖固化。",
      "PostgreSQL 保存身份、修订、权限、位置、关系和任务状态；文件卷保存不可变原件。JSON/Markdown/HTML 是交换或阅读副本。",
      "知识关系图由 PostgreSQL 数据构建；Neo4j 如以后采用，只作为可重建投影。AI 不决定权威知识和验收事实。",
      "C4恢复管理代码与单机补验已推进至Task8限定收尾；Task9和独立完成收据仍开放，不能将单机ReadingCopy交付称为生产服务。"
    ],
    "nodes": [
      {
        "id": "user",
        "col": 0,
        "row": 0,
        "title": "个人学习者",
        "lines": [
          "电脑 / 平板 / 手机",
          "跨设备访问同一服务"
        ],
        "status": "neutral"
      },
      {
        "id": "web",
        "col": 1,
        "row": 0,
        "title": "Web 学习工作台",
        "lines": [
          "阅读 · 编辑 · 图谱 · 今日",
          "P1-A / P1-B 待建设"
        ],
        "status": "planned"
      },
      {
        "id": "sources",
        "col": 2,
        "row": 0,
        "title": "课程与实践来源",
        "lines": [
          "PDF / PPTX / Notebook / 代码",
          "原格式资料与实验产物"
        ],
        "status": "neutral"
      },
      {
        "id": "learning",
        "col": 0,
        "row": 1,
        "title": "学习与计划模块",
        "lines": [
          "单元 / 任务 / 尝试 / 验收",
          "P1-B；规则推荐延伸到 P2"
        ],
        "status": "planned"
      },
      {
        "id": "api",
        "col": 1,
        "row": 1,
        "title": "Rust HTTP / 身份边界",
        "lines": [
          "会话 · DTO · 授权入口",
          "P1-A；Axum 为建议"
        ],
        "status": "planned"
      },
      {
        "id": "import",
        "col": 2,
        "row": 1,
        "title": "交换与导入入口",
        "lines": [
          "C3 快照已验收；资料入口待建",
          "批量更新属于后续阶段"
        ],
        "status": "next"
      },
      {
        "id": "content",
        "col": 0,
        "row": 2,
        "title": "内容与关系内核",
        "lines": [
          "块 / 组合 / 个人层 / 审查",
          "P0-A / B1–B4 已验收"
        ],
        "status": "done"
      },
      {
        "id": "jobs",
        "col": 1,
        "row": 2,
        "title": "持久任务与 Worker",
        "lines": [
          "job_outbox → job → 租约",
          "C2 资产完整性已验收"
        ],
        "status": "done"
      },
      {
        "id": "assetapi",
        "col": 2,
        "row": 2,
        "title": "资产与来源模块",
        "lines": [
          "资源版本 · 精确使用授权",
          "C1 原字节与 v3 图片/附件"
        ],
        "status": "done"
      },
      {
        "id": "pg",
        "col": 0,
        "row": 3,
        "title": "PostgreSQL 权威库",
        "lines": [
          "结构化正文与关系 / 作业状态",
          "SQLx · 事务 · 追加迁移"
        ],
        "status": "done"
      },
      {
        "id": "derived",
        "col": 1,
        "row": 3,
        "title": "可重建派生层",
        "lines": [
          "预览 / OCR / 搜索 / 检索片段",
          "处理器与索引后续按需建设"
        ],
        "status": "optional"
      },
      {
        "id": "files",
        "col": 2,
        "row": 3,
        "title": "不可变原件库",
        "lines": [
          "SHA-256 原件 · 独立持久卷",
          "独立卷不等于独立故障域"
        ],
        "status": "done"
      },
      {
        "id": "portable",
        "col": 0,
        "row": 4,
        "title": "C3 / C4 可移植性",
        "lines": [
          "Task6/7 私有单机限定完成",
          "Task9 与 C4 整关开放"
        ],
        "status": "next"
      },
      {
        "id": "graph",
        "col": 1,
        "row": 4,
        "title": "Neo4j 可选投影",
        "lines": [
          "首版不常驻",
          "来源仍为 PostgreSQL"
        ],
        "status": "optional"
      },
      {
        "id": "ai",
        "col": 2,
        "row": 4,
        "title": "可选 AI / 外部执行",
        "lines": [
          "仅建议；实验默认外部运行",
          "关闭后不影响核心流程"
        ],
        "status": "optional"
      }
    ],
    "edges": [
      {
        "from": "user",
        "to": "web",
        "label": "访问"
      },
      {
        "from": "web",
        "to": "api",
        "label": "同源 API"
      },
      {
        "from": "sources",
        "to": "import",
        "label": "登记 / 受控导入"
      },
      {
        "from": "api",
        "to": "learning",
        "label": "业务调用"
      },
      {
        "from": "api",
        "to": "content",
        "label": "校验与编排"
      },
      {
        "from": "import",
        "to": "assetapi",
        "label": "原件与引用"
      },
      {
        "from": "content",
        "to": "pg",
        "label": "事务读写"
      },
      {
        "from": "assetapi",
        "to": "files",
        "label": "校验 / 定稿 / 授权读取"
      },
      {
        "from": "assetapi",
        "to": "pg",
        "label": "元数据 / 精确使用 / 授权",
        "labelAt": {
          "x": 790,
          "y": 509,
          "anchor": "middle"
        }
      },
      {
        "from": "learning",
        "to": "pg",
        "label": "学习事实",
        "dashed": true,
        "fromPort": "left",
        "toPort": "left",
        "route": [
          [
            40,
            278
          ],
          [
            40,
            594
          ]
        ],
        "labelAt": {
          "x": 45,
          "y": 353,
          "anchor": "start"
        }
      },
      {
        "from": "jobs",
        "to": "pg",
        "label": "转投 / 租约 / 结果"
      },
      {
        "from": "jobs",
        "to": "assetapi",
        "label": "复用精确授权"
      },
      {
        "from": "jobs",
        "to": "derived",
        "label": "未来处理器",
        "dashed": true
      },
      {
        "from": "files",
        "to": "portable",
        "label": "原字节闭包",
        "fromPort": "right",
        "toPort": "bottom",
        "route": [
          [
            1090,
            594
          ],
          [
            1090,
            858
          ],
          [
            217,
            858
          ]
        ]
      },
      {
        "from": "pg",
        "to": "portable",
        "label": "闭包与一致快照"
      },
      {
        "from": "pg",
        "to": "graph",
        "label": "可重建投影",
        "dashed": true
      },
      {
        "from": "derived",
        "to": "ai",
        "label": "按需适配",
        "dashed": true
      }
    ]
  },
  {
    "id": "domain",
    "title": "02 · 块、位置与版本关系",
    "subtitle": "箭头统一由持有者或引用者指向目标；省略部分约束字段，不把对象画成独立服务。",
    "notes": [
      "稳定 ID 与不可变修订 ID 分开；正式发布和阅读快照固定修订，不随 head 自动漂移。相同字节哈希也不会自动合并业务身份。",
      "一次出现由 occurrence 或 placement 标识；正文修订与位置移动是不同操作。个人层固定原文基线，不改写原文。",
      "关系/认识审查属于已实现的知识证据内核；学习任务的尝试、产物与能力验收属于未来 P1-B，不能混为已完成。",
      "当前正文支持 v1 文本、v2 文本/块引用/关系视图、v3 图片/附件。独立公式、代码、表格、练习、评分规则等更多类型仍需各自契约与验收。"
    ],
    "nodes": [
      {
        "id": "block",
        "col": 0,
        "row": 0,
        "title": "Block 稳定身份",
        "lines": [
          "用途 intent 与形式分开",
          "修订时不换稳定 ID"
        ],
        "status": "done"
      },
      {
        "id": "reading",
        "col": 1,
        "row": 0,
        "title": "ReadingViewRevision",
        "lines": [
          "原文 + 个人层 + 关系选择",
          "Original / Fused / Personal"
        ],
        "status": "done"
      },
      {
        "id": "resource",
        "col": 2,
        "row": 0,
        "title": "ResourceVersion / Source",
        "lines": [
          "逻辑资源的不可变版本",
          "页 / 区域 / cell / 时间定位"
        ],
        "status": "done"
      },
      {
        "id": "composition",
        "col": 0,
        "row": 1,
        "title": "Composition / Release",
        "lines": [
          "固定有序组合与原子发布",
          "复用引用，不复制正文"
        ],
        "status": "done"
      },
      {
        "id": "overlay",
        "col": 1,
        "row": 1,
        "title": "OverlayRevision 个人层",
        "lines": [
          "固定根文档和原文基线",
          "独立位置与个人块引用"
        ],
        "status": "done"
      },
      {
        "id": "relation",
        "col": 2,
        "row": 1,
        "title": "RelationRevision 语义边",
        "lines": [
          "端点固定块修订",
          "支持 / 反对 / 启发等"
        ],
        "status": "done"
      },
      {
        "id": "occurrence",
        "col": 0,
        "row": 2,
        "title": "Occurrence 精确出现",
        "lines": [
          "有序项 / 完整出现路径",
          "同一块可重复出现"
        ],
        "status": "done"
      },
      {
        "id": "placement",
        "col": 1,
        "row": 2,
        "title": "Placement / GapAnchor",
        "lines": [
          "左右邻居 / 附着偏好",
          "未确定迁移项保持可找回"
        ],
        "status": "done"
      },
      {
        "id": "review",
        "col": 2,
        "row": 2,
        "title": "EpistemicReview 认识审查",
        "lines": [
          "固定猜想与证据快照",
          "结论有条件；撤权后不泄露"
        ],
        "status": "done"
      },
      {
        "id": "learning",
        "col": 0,
        "row": 3,
        "title": "学习事实与能力判断",
        "lines": [
          "LearningRun / Plan / Attempt",
          "规则、产物和验收；P1-B"
        ],
        "status": "planned"
      },
      {
        "id": "revision",
        "col": 1,
        "row": 3,
        "title": "BlockRevision 不可变正文",
        "lines": [
          "精确 block_id + revision_id",
          "文本 / 引用 / 图片 / 附件"
        ],
        "status": "done"
      },
      {
        "id": "asset",
        "col": 2,
        "row": 3,
        "title": "Asset + 原件字节",
        "lines": [
          "业务资产 ID ≠ 内容哈希",
          "精确 AssetUse 授权后读取"
        ],
        "status": "done"
      }
    ],
    "edges": [
      {
        "from": "block",
        "to": "revision",
        "label": "持有修订历史",
        "fromPort": "left",
        "toPort": "bottom",
        "route": [
          [
            42,
            120
          ],
          [
            42,
            704
          ],
          [
            567,
            704
          ]
        ]
      },
      {
        "from": "reading",
        "to": "composition",
        "label": "固定原文基线"
      },
      {
        "from": "reading",
        "to": "overlay",
        "label": "固定个人层"
      },
      {
        "from": "reading",
        "to": "relation",
        "label": "固定关系选择"
      },
      {
        "from": "composition",
        "to": "occurrence",
        "label": "固定有序成员"
      },
      {
        "from": "overlay",
        "to": "placement",
        "label": "固定个人位置"
      },
      {
        "from": "occurrence",
        "to": "revision",
        "label": "精确块引用"
      },
      {
        "from": "placement",
        "to": "revision",
        "label": "个人块引用"
      },
      {
        "from": "relation",
        "to": "revision",
        "label": "固定端点",
        "fromPort": "right",
        "toPort": "bottom",
        "route": [
          [
            1090,
            278
          ],
          [
            1090,
            668
          ],
          [
            567,
            668
          ]
        ]
      },
      {
        "from": "review",
        "to": "relation",
        "label": "证据关系选择"
      },
      {
        "from": "resource",
        "to": "asset",
        "label": "固定原件",
        "fromPort": "left",
        "toPort": "left",
        "route": [
          [
            742,
            120
          ],
          [
            742,
            594
          ]
        ],
        "labelAt": {
          "x": 752,
          "y": 353,
          "anchor": "start"
        }
      },
      {
        "from": "revision",
        "to": "asset",
        "label": "v3 精确资产使用"
      },
      {
        "from": "learning",
        "to": "reading",
        "label": "固定内容上下文",
        "dashed": true,
        "fromPort": "left",
        "toPort": "top",
        "route": [
          [
            16,
            594
          ],
          [
            16,
            20
          ],
          [
            567,
            20
          ]
        ],
        "labelAt": {
          "x": 290,
          "y": 14,
          "anchor": "middle"
        }
      }
    ]
  },
  {
    "id": "crates",
    "title": "03 · Rust 代码与授权边界",
    "subtitle": "实线显示现有模块依赖与运行接口；虚线表示未来接入。crate 是代码模块，不是独立网络服务。",
    "notes": [
      "当前a5eb28d Cargo workspace有core、assets、db、worker、backup五个crate。真实直接依赖：backup → db/assets/core；worker → db/assets/core；db → assets/core；assets → core。assets不负责业务actor授权。API与Web尚未加入workspace。",
      "Worker 原件与二进制只读，staging 私有可写；C3 另挂服务所有的私有可写 snapshots 卷，管理凭据不进入普通 Worker。",
      "数据库 runtime 是可信内部服务角色，非每位最终用户或每个 Worker 的独立数据库租户。用户权限由业务 actor、当前空间授权和必要引用闭包共同判定。",
      "两项 P2 契约缺口已记录：持当前 token 的共享 runtime 可调用通用 succeed 而不写专用结果；也可直接调用完成函数绕过 Rust 字节复核。正常 Worker 已验收路径使用原子结果函数。这里 P2 是问题优先级，不表示已安排在路线 P2 阶段。",
      "旧 outbox_event 只保存 composition_released，C2 的 job_outbox / job 是独立协议；图与实现均不把两者合并成一张通用队列表。",
      "图省略backup→core直接连线以保持可读，代码依赖以Cargo.toml为准。"
    ],
    "nodes": [
      {
        "id": "web",
        "col": 0,
        "row": 0,
        "title": "apps/web 待建设",
        "lines": [
          "编辑草稿与用户交互",
          "不直连数据库或文件目录"
        ],
        "status": "planned"
      },
      {
        "id": "api",
        "col": 1,
        "row": 0,
        "title": "learning-api 待建设",
        "lines": [
          "HTTP 身份 → 业务 actor",
          "稳定 DTO / 幂等与冲突语义"
        ],
        "status": "planned"
      },
      {
        "id": "worker",
        "col": 2,
        "row": 0,
        "title": "learning-worker",
        "lines": [
          "进程循环 / 处理器 / 停止恢复",
          "非 owner runtime 登录"
        ],
        "status": "done"
      },
      {
        "id": "core",
        "col": 0,
        "row": 1,
        "title": "learning-core",
        "lines": [
          "版本化契约 / 校验 / 摘要",
          "块、位置、关系、资产、任务"
        ],
        "status": "done"
      },
      {
        "id": "db",
        "col": 1,
        "row": 1,
        "title": "learning-db",
        "lines": [
          "SQLx / 显式事务 / 授权查询",
          "发布、融合视图、关系、作业"
        ],
        "status": "done"
      },
      {
        "id": "assets",
        "col": 2,
        "row": 1,
        "title": "learning-assets",
        "lines": [
          "不可变字节 / staging / 校验",
          "Filesystem 原件存储"
        ],
        "status": "done"
      },
      {
        "id": "pg",
        "col": 1,
        "row": 2,
        "title": "PostgreSQL 内部接口",
        "lines": [
          "最小权限 / 限定数据库函数",
          "业务行、收据、两个独立 outbox"
        ],
        "status": "done"
      },
      {
        "id": "file",
        "col": 2,
        "row": 2,
        "title": "文件系统存储卷",
        "lines": [
          "原件只读；staging 私有可写",
          "C3 挂私有可写 snapshots"
        ],
        "status": "done"
      },
      {
        "id": "backup",
        "col": 0,
        "row": 2,
        "title": "learning-backup",
        "lines": [
          "Task6/7 私有范围限定完成",
          "Task9 与公开恢复仍开放"
        ],
        "status": "next"
      }
    ],
    "edges": [
      {
        "from": "web",
        "to": "api",
        "label": "JSON API",
        "dashed": true
      },
      {
        "from": "api",
        "to": "db",
        "label": "身份和命令",
        "dashed": true
      },
      {
        "from": "worker",
        "to": "db",
        "label": "作业与授权",
        "labelAt": {
          "x": 870,
          "y": 194,
          "anchor": "middle"
        }
      },
      {
        "from": "worker",
        "to": "assets",
        "label": "核对原件"
      },
      {
        "from": "worker",
        "to": "core",
        "label": "契约",
        "route": [
          [
            1088,
            118
          ],
          [
            1088,
            208
          ],
          [
            45,
            208
          ],
          [
            45,
            270
          ],
          [
            70,
            270
          ]
        ],
        "fromPort": "right",
        "toPort": "left",
        "labelAt": {
          "x": 150,
          "y": 200,
          "anchor": "middle"
        }
      },
      {
        "from": "db",
        "to": "core",
        "label": "类型 / 校验"
      },
      {
        "from": "db",
        "to": "assets",
        "label": "字节先就绪"
      },
      {
        "from": "assets",
        "to": "core",
        "label": "快照包契约",
        "fromPort": "bottom",
        "toPort": "bottom",
        "route": [
          [
            917,
            355
          ],
          [
            217,
            355
          ]
        ],
        "labelAt": {
          "x": 567,
          "y": 367,
          "anchor": "middle"
        }
      },
      {
        "from": "db",
        "to": "pg",
        "label": "参数化 SQL"
      },
      {
        "from": "assets",
        "to": "file",
        "label": "流式 I/O"
      },
      {
        "from": "backup",
        "to": "db",
        "label": "直接依赖",
        "fromPort": "top",
        "toPort": "bottom",
        "route": [
          [
            217,
            347
          ],
          [
            567,
            347
          ]
        ],
        "labelAt": {
          "x": 392,
          "y": 347,
          "anchor": "middle"
        }
      },
      {
        "from": "backup",
        "to": "assets",
        "label": "直接依赖",
        "fromPort": "left",
        "toPort": "right",
        "route": [
          [
            25,
            436
          ],
          [
            25,
            525
          ],
          [
            1095,
            525
          ],
          [
            1095,
            278
          ]
        ],
        "labelAt": {
          "x": 567,
          "y": 518,
          "anchor": "middle"
        }
      },
      {
        "from": "backup",
        "to": "pg",
        "label": "受控管理",
        "labelAt": {
          "x": 392,
          "y": 376,
          "anchor": "middle"
        }
      }
    ]
  },
  {
    "id": "jobs",
    "title": "04 · 持久任务与真实故障恢复",
    "subtitle": "C2 已验收路径：新事件来自精确 v3 块资产使用，采用至少一次执行与幂等结果。",
    "notes": [
      "仅登记资产、仅关联资源版本或旧发布 outbox，不会自动创建这类资产检查任务。正文和事件同事务提交；旧请求收据重放不增事件。",
      "claim 使用 PostgreSQL 时钟和数据库生成 token。续租、检查点、成功和失败都校验当前 token；最多三次尝试，取消与终态不复活。",
      "专用 Worker 只挂 runtime secret、只读资产与只读二进制；真实 SIGKILL 后第二 Worker 接管 attempt 2，旧 token 的四种写入均被拒绝。",
      "完整性处理只核大小、SHA-256 与轻量元信息，不宣称已实现预览、OCR、完整媒体解析或恶意文件扫描。",
      "C4 Task6/7恢复时会使旧租约/token失效，对已提交效果、确认无效果与未知效果分别处理；Unknown/HOLD不会自动变成可运行。其证据范围限规定私有单机场景。"
    ],
    "nodes": [
      {
        "id": "write",
        "col": 0,
        "row": 0,
        "title": "真实 v3 块写入",
        "lines": [
          "Figure / Attachment",
          "request_id 幂等"
        ],
        "status": "done"
      },
      {
        "id": "tx",
        "col": 1,
        "row": 0,
        "title": "同一业务事务",
        "lines": [
          "BlockRevision + block_asset_use",
          "同时写 job_outbox"
        ],
        "status": "done"
      },
      {
        "id": "outbox",
        "col": 2,
        "row": 0,
        "title": "job_outbox 持久事件",
        "lines": [
          "actor + 精确 BlockRef",
          "处理器版本 / 稳定业务键"
        ],
        "status": "done"
      },
      {
        "id": "handler",
        "col": 0,
        "row": 1,
        "title": "资产完整性处理器",
        "lines": [
          "按原 actor 重验当前授权",
          "核对大小与 SHA-256"
        ],
        "status": "done"
      },
      {
        "id": "lease",
        "col": 1,
        "row": 1,
        "title": "Worker 原子领取租约",
        "lines": [
          "数据库时间 + 新 token",
          "attempt / lease_until"
        ],
        "status": "done"
      },
      {
        "id": "job",
        "col": 2,
        "row": 1,
        "title": "原子转投为唯一 job",
        "lines": [
          "事件与作业一一收敛",
          "重复 / 并发转投不增任务"
        ],
        "status": "done"
      },
      {
        "id": "complete",
        "col": 0,
        "row": 2,
        "title": "带 token 的原子完成",
        "lines": [
          "写 asset_integrity_result",
          "同事务标记 succeeded"
        ],
        "status": "done"
      },
      {
        "id": "takeover",
        "col": 1,
        "row": 2,
        "title": "进程终止后的接管",
        "lines": [
          "真实 SIGKILL → 租约到期",
          "第二 Worker 领取新 token"
        ],
        "status": "done"
      },
      {
        "id": "terminal",
        "col": 2,
        "row": 2,
        "title": "取消 / 撤权 / 有限失败",
        "lines": [
          "cancelled / failed / retry_wait",
          "失败不撤销已发布原文"
        ],
        "status": "done"
      },
      {
        "id": "result",
        "col": 0,
        "row": 3,
        "title": "唯一、版本化结果",
        "lines": [
          "重复运行不增结果",
          "保留输入与输出摘要"
        ],
        "status": "done"
      },
      {
        "id": "fence",
        "col": 1,
        "row": 3,
        "title": "拒绝旧 Worker 写入",
        "lines": [
          "renew / checkpoint",
          "succeed / fail 均拒绝"
        ],
        "status": "done"
      },
      {
        "id": "old",
        "col": 2,
        "row": 3,
        "title": "旧 outbox_event 独立",
        "lines": [
          "只处理 composition_released",
          "不是 C2 作业输入"
        ],
        "status": "neutral"
      }
    ],
    "edges": [
      {
        "from": "write",
        "to": "tx",
        "label": "校验后提交"
      },
      {
        "from": "tx",
        "to": "outbox",
        "label": "原子持久化"
      },
      {
        "from": "outbox",
        "to": "job",
        "label": "事务转投"
      },
      {
        "from": "job",
        "to": "lease",
        "label": "领取"
      },
      {
        "from": "lease",
        "to": "handler",
        "label": "精确输入"
      },
      {
        "from": "handler",
        "to": "complete",
        "label": "处理成功"
      },
      {
        "from": "complete",
        "to": "result",
        "label": "提交结果"
      },
      {
        "from": "lease",
        "to": "takeover",
        "label": "进程丢失"
      },
      {
        "from": "takeover",
        "to": "fence",
        "label": "新租约使旧 token 失效"
      },
      {
        "from": "lease",
        "to": "terminal",
        "label": "受控终止 / 重试"
      }
    ]
  },
  {
    "id": "portability",
    "title": "05 · C3 交换与 C4 恢复",
    "subtitle": "C3范围快照交换已验收；C4单机链已完成Task8限定回归与审查；Task9独立域和正常公开恢复仍待完成。",
    "notes": [
      "C3 Task 1–7 已完成分项与整关隔离验收：Task 7 在全新 Compose 项目通过完整 workspace、四套旧版升级、真实 Worker SIGKILL/租约接管、双端往返和四组失败注入。独立 root 只读审计核验 602 份证据文件、341 个源码文件、301 项镜像输入；状态为 P0_C3_VERIFIED / NOT_PRODUCTION。",
      "C3 输入为精确 ReadingViewRevision、模式和原件选项；当前授权求闭包，作业结束与交付时再次检查。已下载的文件无法被服务器远程撤回。",
      "无损交换以版本化 JSON 与 manifest 为准；HTML/Markdown 是阅读副本。导入同 ID 同字节幂等复用，不同字节拒绝。",
      "C4 Task2/3/5分别在原固定源码上完成规定单机补验；Task4只覆盖内容三门。Task6的2e5026d私有16/16与Task7的36c89f2私有6/6分别留证，不作为当前全工作区或四套升级通过。",
      "Task7六案：完整正例、缺失原件、损坏dump、传输中断、未知效果、原件根替换。未知效果在导入后拒绝并留3项UNKNOWN/runnable0；不是导入前目标不变。根替换只声明列举catalog/assets/attempt字节不变，不声明整个文件系统不变。",
      "三个good-pin恢复路径各四份ReadingCopy文件共1172B，四处逐字相等且inode独立；它们位于同一主机，不构成独立故障域。失败53df不计通过，历史未观测的外层OS收据不补造。",
      "Task8规定私有回归及整组审查已限定完成。Task9仍须以真实独立目标、托管密钥及首签/新发布权限集成完成正常CompleteBackup与源端不可访问恢复；C4整关保持开放。",
      "未来P1-B新增学习任务、尝试、产物、验收与计划历史后，须同步扩展恢复闭包并另验整链。"
    ],
    "nodes": [
      {
        "id": "view",
        "col": 0,
        "row": 0,
        "title": "C3 · 选定阅读快照",
        "lines": [
          "精确版本 / 模式 / 原件选项",
          "按当前主体权限取范围"
        ],
        "status": "done"
      },
      {
        "id": "closure",
        "col": 1,
        "row": 0,
        "title": "只读一致快照求闭包",
        "lines": [
          "块 / 位置 / 关系 / 审查 / 资产",
          "不泄露隐藏 ID 或计数"
        ],
        "status": "done"
      },
      {
        "id": "bundle",
        "col": 2,
        "row": 0,
        "title": "manifest + JSON + 原件",
        "lines": [
          "逐对象摘要与格式版本",
          "HTML / Markdown 阅读副本"
        ],
        "status": "done"
      },
      {
        "id": "visible",
        "col": 0,
        "row": 1,
        "title": "原子发布导入批次",
        "lines": [
          "同 ID 同字节复用",
          "失败无半可用视图"
        ],
        "status": "done"
      },
      {
        "id": "preflight",
        "col": 1,
        "row": 1,
        "title": "staging 校验与预检",
        "lines": [
          "引用闭包 / 权限归属 / 路径",
          "摘要冲突或缺件即拒绝"
        ],
        "status": "done"
      },
      {
        "id": "delivery",
        "col": 2,
        "row": 1,
        "title": "重验权限后交付",
        "lines": [
          "完成时及下载时再检查",
          "导入授权目标范围"
        ],
        "status": "done"
      },
      {
        "id": "pause",
        "col": 0,
        "row": 2,
        "title": "C4 · 受控写入维护窗",
        "lines": [
          "Task2 源端与监督单机收尾",
          "高权管理侧；非公开API"
        ],
        "status": "next"
      },
      {
        "id": "backup",
        "col": 1,
        "row": 2,
        "title": "一致数据库 + 资产清单",
        "lines": [
          "Task2/3 单机补验收尾",
          "独立完成收据归 Task9"
        ],
        "status": "next"
      },
      {
        "id": "copy",
        "col": 2,
        "row": 2,
        "title": "独立故障域验收",
        "lines": [
          "单机传输专项已通过",
          "云目标已盘点，密钥未安装"
        ],
        "status": "planned"
      },
      {
        "id": "check",
        "col": 0,
        "row": 3,
        "title": "完整恢复资格校验",
        "lines": [
          "Task6 私有单机16/16",
          "Task7 三路 P6/字节独核"
        ],
        "status": "next"
      },
      {
        "id": "restore",
        "col": 1,
        "row": 3,
        "title": "干净目标导入与原件",
        "lines": [
          "Task5 规定单机补验完成",
          "公开恢复资格仍未签发"
        ],
        "status": "next"
      },
      {
        "id": "future",
        "col": 2,
        "row": 3,
        "title": "P1-B 扩展恢复闭包",
        "lines": [
          "学习事实 / 规则 / 产物 / 计划",
          "另做全链恢复验收"
        ],
        "status": "planned"
      }
    ],
    "edges": [
      {
        "from": "view",
        "to": "closure",
        "label": "固定输入"
      },
      {
        "from": "closure",
        "to": "bundle",
        "label": "精确清单"
      },
      {
        "from": "bundle",
        "to": "delivery",
        "label": "授权交付"
      },
      {
        "from": "delivery",
        "to": "preflight",
        "label": "受控导入"
      },
      {
        "from": "preflight",
        "to": "visible",
        "label": "校验全通过"
      },
      {
        "from": "pause",
        "to": "backup",
        "label": "一致恢复点"
      },
      {
        "from": "backup",
        "to": "copy",
        "label": "不可变资产复制"
      },
      {
        "from": "copy",
        "to": "restore",
        "label": "校验完整备份",
        "labelAt": {
          "x": 872,
          "y": 508,
          "anchor": "middle"
        }
      },
      {
        "from": "restore",
        "to": "check",
        "label": "恢复验证"
      },
      {
        "from": "future",
        "to": "restore",
        "label": "补学习事实",
        "dashed": true
      }
    ]
  },
  {
    "id": "deployment",
    "title": "06 · 目标部署与存储边界",
    "subtitle": "目标为私有单机 Linux + Compose。现有证明来自隔离验收环境，未部署成生产服务。",
    "notes": [
      "C4 源控制根绑定已在单可信容器命名空间实测；独立签发/每部署编译 pin 不构成宿主生产安装流程，独立存储故障域仍延后。",
      "只有 HTTPS 入口面向浏览器；数据库、文件卷和高权管理入口不直接公开。API 与 Worker 复用业务代码而采用不同运行职责。",
      "C2 专用 Worker 容器以非 root 运行、只读根文件系统、私有 tmpfs，限制资源；管理 secret 不在 Worker 文件系统中。宿主 root / Docker daemon 仍属于可信管理边界。",
      "assets 是权威原字节；staging 是未完成工作；derived 是可重建派生物；验收 evidence 保存此次测试证据，并不等于将来学习产物的数据库模型。",
      "生产部署前重新核对服务器时钟、runtime配置、HTTP授权、已记录的内部runtime契约缺口、日志/容量/升级/备份策略。当前没有本轮生产时钟或服务健康实测，不将历史故障写成实时状态。"
    ],
    "nodes": [
      {
        "id": "browser",
        "col": 0,
        "row": 0,
        "title": "浏览器 / HTTPS",
        "lines": [
          "唯一面向用户的入口",
          "电脑 / 平板 / 手机"
        ],
        "status": "planned"
      },
      {
        "id": "gateway",
        "col": 1,
        "row": 0,
        "title": "反向代理 / TLS",
        "lines": [
          "同源静态 Web 与 API",
          "P1-A 部署时配置"
        ],
        "status": "planned"
      },
      {
        "id": "management",
        "col": 2,
        "row": 0,
        "title": "受控运维入口",
        "lines": [
          "部署 / 迁移 / 恢复管理",
          "高权凭据单独保管"
        ],
        "status": "planned"
      },
      {
        "id": "web",
        "col": 0,
        "row": 1,
        "title": "Web 静态资源",
        "lines": [
          "受控内容渲染 / 草稿缓冲",
          "不执行 Notebook"
        ],
        "status": "planned"
      },
      {
        "id": "api",
        "col": 1,
        "row": 1,
        "title": "API 运行角色",
        "lines": [
          "服务端身份与业务授权",
          "后续独立进程"
        ],
        "status": "planned"
      },
      {
        "id": "worker",
        "col": 2,
        "row": 1,
        "title": "Worker 运行角色",
        "lines": [
          "C2 容器隔离已验收",
          "仅 runtime secret / 只读资产"
        ],
        "status": "done"
      },
      {
        "id": "staging",
        "col": 0,
        "row": 2,
        "title": "staging / derived",
        "lines": [
          "临时工作 / 可重建派生缓存",
          "各自寿命，不冒充权威正文"
        ],
        "status": "neutral"
      },
      {
        "id": "pg",
        "col": 1,
        "row": 2,
        "title": "内部 PostgreSQL",
        "lines": [
          "postgres 独立持久卷",
          "业务事实与持久任务"
        ],
        "status": "done"
      },
      {
        "id": "assets",
        "col": 2,
        "row": 2,
        "title": "assets 独立持久卷",
        "lines": [
          "不可变原字节 / SHA-256",
          "当前为 filesystem 后端"
        ],
        "status": "done"
      },
      {
        "id": "backup",
        "col": 1,
        "row": 3,
        "title": "C4 备份协调器",
        "lines": [
          "维护窗与一致性清单",
          "管理权限与运行角色分开"
        ],
        "status": "next"
      },
      {
        "id": "remote",
        "col": 2,
        "row": 3,
        "title": "独立备份副本",
        "lines": [
          "另一故障域 / 可校验保留集",
          "密钥另存；恢复到干净实例"
        ],
        "status": "planned"
      }
    ],
    "edges": [
      {
        "from": "browser",
        "to": "gateway",
        "label": "HTTPS"
      },
      {
        "from": "gateway",
        "to": "web",
        "label": "静态资源"
      },
      {
        "from": "gateway",
        "to": "api",
        "label": "受保护 API"
      },
      {
        "from": "management",
        "to": "worker",
        "label": "发布固定构建"
      },
      {
        "from": "api",
        "to": "pg",
        "label": "私有网络"
      },
      {
        "from": "worker",
        "to": "pg",
        "label": "受限作业权限"
      },
      {
        "from": "worker",
        "to": "assets",
        "label": "只读"
      },
      {
        "from": "api",
        "to": "staging",
        "label": "受控上传 / 导出"
      },
      {
        "from": "pg",
        "to": "backup",
        "label": "一致备份"
      },
      {
        "from": "assets",
        "to": "remote",
        "label": "按资产清单复制",
        "dashed": true
      },
      {
        "from": "backup",
        "to": "remote",
        "label": "数据库 + 清单",
        "dashed": true
      }
    ]
  }
];

export const meta = {
  "title": "知织 · KnowWeave｜实施路线与系统架构",
  "date": "2026-10-29",
  "baseline": "3342c0554c18787749d0f06a772743f867c18720",
  "sourceCommit": "63a66327f032f01201eb3c4d6d6d60c0894a81bf",
  "state": "C4实施中 · DB3限定普通检查通过，Root／M3接线继续",
  "progressSummary": "DB3六个既有源码叶完成限定普通Linux13门：DB默认62通过／24忽略、peer117通过／26忽略，均0失败／0过滤；peer新增12个独立普通正文通过，配置间重复正文不相加。backup默认与offhost只有依赖check。真实Root发行、M3整个future／finally、M5、C11整链、S5封闭管线与Task9／C4验收仍开放。",
  "progressEvidence": "批次2fb62369-09c1-41fa-8126-9d73b12fe40f，result 1dbfec0f2548f14f601b8898ea2e7dd6d35577570f795328c188a909de5a33bc；evidence ZIP 0f6eb27597aeaf456764c47771dfa77323dce89e4113c86fb22b9986b7316c6b。源CI 546f0cded8c37a685e914874610cfbd80aa0f7e02400562804caac8aa2d018a9／MF 922ebbee717c11ba9256a4f617b47c205412fc166bb90bc4d477ed20b41758e1／TEST 5882ad2f4be6eeff7e30c7d9e35f362dbf25d751f4aba7d4a4635966e0531cb5，Root CODE b45411a2b78eff8448a77456c5b37e15e8f17818b27374654c41d39ddd64f651。最终checks CONTROL 0c97cc470c8e51a20bb24812a9fbca6436d13214087df8b509f63e8ab1be39ab；独立完整原件OTHER 296108a050d5e17956f511b342c0f5df9fccd7d2e333ffbfecdcdbbcb117ad6c／INDEX bdd755139f3112a3d9d6501c4d86ecbbef2ab2176841b39eae125cc92a370322。实际13门全exit0、完整86／143名单与24／26顶层ignored、共享历史资源和新12正文，按该独审有限范围核证；源码原件未变、精确CID自然退出及清理、返回证据链已核。不是独立远程亲见、连续OS cap或全链资源计费证明。",
  "pendingSummary": "真实Root持有者、全host before枚举、实际genesis／after、持久leaf与ACK生产者仍待实现；M3整个actor future／finally、Source首S与Publication阶段／capture-only admission差异、M5保持仍开放。S5下游sealed／fs等读链的完整有界计费及cap inventory未闭合，缺口清单仍不完整。37叶是本轮补充编码的范围，并不把范围外实现概括为未授权；需要扩大本轮施工范围时应逐项列明。现场密钥／信任／launcher安装、PG／安全权限、源隔离／恢复和真实整链按具体方案办理。",
  "historicalMaintenance": "各历史来源、失败UUID、HOLD与未观测字段保留，不将旧失败改写为通过，不重放旧资源，也不把不同源码的测试数相加。Task5受控11/11是10项历史c749＋1项85c；另有85c九项native及最终回归独审采用，限定代码与规定单机补验完成。Task6的16/16属于2e5026d；Task7的6/6属于36c89f2。历史检查点以Git历史和验收清单的折叠章节为准。",
  "intro": "面向个人长期学习、支持多领域复用的服务器应用。以可独立修订的块为核心，将原文、笔记、图片、想法与猜想连续编排；版本、位置、关系、证据与学习事实各自留存。",
  "scope": "当前状态与目标设计同图展示：P0-A、B1–B4、C1–C3已分关验收；C4代码和规定私有单机补验已完成Task8限定回归收尾，Task9及C4整关仍开放。颜色只表示标注范围，不代表部署。",
  "snapshotSummary": "A5 Rust1052通过/0失败/169忽略，四套真实旧版升级及15迁移checksum已过；ordinary Python824/82跳过，Root903/3跳过，合并904个unique Linux正文，deploy21。54 fresh、6历史桥、33RootFS和5FS辅助分列；两private Linux frame opt-in及Windows parser边界保留。",
  "boundarySummary": "Task9、C4整关、独立存储、正常CompleteBackup、RecoveredInstance、公开恢复和生产仍OPEN。",
  "roadmapHeadline": "当前进度 · DB3固定控制动作普通检查通过，Root／M3继续接线",
  "taskPlanScope": "已批37叶内本片6个既有DB源码叶／三个固定动作；peer新12普通正文；仅DB普通13门，backup依赖check，整体Root／M3／M5／S5／C11保持开放。",
  "nextSteps": [
    [
      "01 · 审核限定发表并继续实际owner接线",
      "精确发布输入与PLAN独审后才允许feature发表；真实Root与M3原future仍需后续实现。"
    ],
    [
      "02 · 连接同原Root与M3实际持有者",
      "真实child／pipes／records／ACK生产和整个future finally；不放宽capture-only谓词。"
    ],
    [
      "03 · 继续S5与现场整链",
      "逐项清点封闭计费、cap inventory、Source／D、M5与receiver；具体现场动作按方案办理。"
    ]
  ]
};

export const principles = [
  [
    "权威分层",
    "PostgreSQL 是结构化业务权威；assets 是原字节权威；派生缓存、导出阅读副本和未来 Neo4j 投影均可重建或重新生成。"
  ],
  [
    "版本不漂移",
    "Block 稳定 ID、修订 ID、组合发布、个人层、关系选择与学习快照分别留存。更新产生候选；是否采用新版由明确操作决定。"
  ],
  [
    "授权贯穿链路",
    "正文、必要引用、位置、关系、影响路径和精确资产使用都核对当前权限；HTTP、搜索与导出必须在后续阶段继承并另验此语义。"
  ],
  [
    "知识证据与学习证据分开",
    "支持/反对关系和认识审查已有内核；学习任务、尝试、产物、评分规则、能力验收尚属 P1-B。阅读完成不会自动成为通过。"
  ],
  [
    "模块化单体",
    "crate 分职责，API 与 Worker 可分进程；首版无需 Kafka、Redis、Kubernetes、向量数据库或常驻 Neo4j。"
  ],
  [
    "生产状态诚实",
    "历史分关、当前源码前置、规定私有单机验收、公开恢复资格和生产部署分别标注。Task6/7有限范围完成；Task8限定完成；Task9与C4整关开放。P1产品入口尚未实施，GitHub候选分支更新不代表main合并或部署。"
  ]
];

export const references = [
  [
    "架构设计文档集（设计参考，状态以本图为准）",
    "docs/KnowWeave架构设计总览.md"
  ],
  [
    "批准记录与阶段边界",
    "docs/设计批准与实施路线.md"
  ],
  [
    "总体设计",
    "docs/学习系统设计方案.md"
  ],
  [
    "块与存储规范",
    "docs/内容分块与存储规范.md"
  ],
  [
    "连续阅读与思考块",
    "docs/连续阅读与思考块设计.md"
  ],
  [
    "当前Task7验收与范围",
    "docs/p0c4-completion-acceptance.md"
  ],
  [
    "Task8当前回归与待验边界",
    "docs/p0c4-task8-current-regression.md"
  ],
  [
    "本轮全路线审核记录",
    "docs/路线图更新审核记录-2026-10-25.md"
  ],
  [
    "Task2/3/5历史补验台账",
    "https://github.com/MansurnamedZhang/learning-system-p0c4/blob/addf1193c98e18c6eadb548ed635c35eaff95707/docs/P0-C4补验与收尾.md"
  ],
  [
    "Task6历史采用与项目进度",
    "https://github.com/MansurnamedZhang/learning-system-p0c4/blob/addf1193c98e18c6eadb548ed635c35eaff95707/docs/项目最新进度.md"
  ],
  [
    "C4九任务补齐施工单",
    "https://github.com/MansurnamedZhang/learning-system-p0c4/blob/addf1193c98e18c6eadb548ed635c35eaff95707/docs/superpowers/plans/2026-10-04-p0c4-completion.md"
  ],
  [
    "C4补齐设计",
    "https://github.com/MansurnamedZhang/learning-system-p0c4/blob/addf1193c98e18c6eadb548ed635c35eaff95707/docs/superpowers/specs/2026-10-04-p0c4-completion-design.md"
  ],
  [
    "C4原五任务父计划",
    "https://github.com/MansurnamedZhang/learning-system-p0c4/blob/addf1193c98e18c6eadb548ed635c35eaff95707/docs/superpowers/plans/2026-09-28-p0c4-backup-recovery.md"
  ],
  [
    "C3整关验证台账",
    "https://github.com/MansurnamedZhang/learning-system-p0c4/blob/addf1193c98e18c6eadb548ed635c35eaff95707/docs/p0c3-verification.md"
  ],
  [
    "当前任务源码workspace",
    "https://github.com/MansurnamedZhang/learning-system-p0c4/tree/feat/task7-attention-recovery"
  ],
  [
    "Neo4j首版决策",
    "docs/数据库选型实验审阅与首版决策.md"
  ],
  [
    "Task9准备、授权缺口与最小审批边界",
    "docs/p0c4-task9-readiness.md"
  ],
  [
    "B1最小集成与失败测试草案",
    "docs/superpowers/specs/2026-10-26-task9-b1-integration-preparation.md"
  ],
  [
    "37条Task9集成测试规格（未执行）",
    "docs/task9/b1-preparation-test-matrix.json"
  ],
  [
    "Task9本地准备收尾、导出复用与剩余阻碍",
    "docs/task9/local-preparation-closeout.md"
  ],
  [
    "Task9目标事实、最小审批与八步执行准备单",
    "docs/task9/execution-readiness.md"
  ],
  [
    "Task9已选目标、FS预检与Linux帧准备原件",
    "docs/task9/selected-target-preparation.md"
  ],
  [
    "Task9异机活操作RFC（历史候选）",
    "docs/task9/b1-offhost-admission-rfc.md"
  ],
  [
    "Task9最终决定原提案（历史封存字节）",
    "docs/task9/b1-final-decision-contract-proposal.md"
  ],
  [
    "Task9最终决定语义采纳记录与实施边界",
    "docs/task9/b1-final-decision-adoption.md"
  ],
  [
    "Task9 R2b准确编码范围、预算与分层验证计划",
    "docs/superpowers/plans/2026-10-26-task9-r2b-implementation.md"
  ],
  [
    "Task9事务借用接缝、当前授权与未验层",
    "docs/task9/r2b-implementation-progress.md"
  ]
];

export const stages = [
  {
    "id": "P0-A",
    "phase": "内容基础",
    "name": "块修订与权限",
    "status": "done",
    "deliver": "稳定块身份、不可变修订、当前授权、幂等请求和并发冲突。",
    "gate": "真实 PostgreSQL 权限、重复写入、冲突与回滚验证。",
    "evidence": "29/29；已验收",
    "link": "https://github.com/MansurnamedZhang/learning-system-p0c4/blob/addf1193c98e18c6eadb548ed635c35eaff95707/docs/p0a-verification.md"
  },
  {
    "id": "P0-B1",
    "phase": "内容基础",
    "name": "组合与原子发布",
    "status": "done",
    "deliver": "固定组合、重复出现的独立 occurrence、精确版本引用及原子发布。",
    "gate": "两文档复用一个块；只升级指定组合；旧发布仍可复现。",
    "evidence": "68/68；已验收",
    "link": "https://github.com/MansurnamedZhang/learning-system-p0c4/blob/addf1193c98e18c6eadb548ed635c35eaff95707/docs/p0b1-verification.md"
  },
  {
    "id": "P0-B2",
    "phase": "内容基础",
    "name": "个人层与融合阅读",
    "status": "done",
    "deliver": "间隙插入、个人位置、原文/融合/个人投影及固定阅读修订。",
    "gate": "位置与正文同步保存；移动不改原文；无法迁移的内容可找回。",
    "evidence": "105/105；已验收",
    "link": "https://github.com/MansurnamedZhang/learning-system-p0c4/blob/addf1193c98e18c6eadb548ed635c35eaff95707/docs/p0b2-verification.md"
  },
  {
    "id": "P0-B3",
    "phase": "内容基础",
    "name": "关系与认识审查",
    "status": "done",
    "deliver": "支持/反对等关系、猜想版本与证据判断、派生/拆分/合并谱系。",
    "gate": "隐藏证据不泄露；猜想新版不自动继承旧判断。",
    "evidence": "222/222；已验收",
    "link": "https://github.com/MansurnamedZhang/learning-system-p0c4/blob/addf1193c98e18c6eadb548ed635c35eaff95707/docs/p0b3-verification.md"
  },
  {
    "id": "P0-B4",
    "phase": "内容基础",
    "name": "读取投影与影响查询",
    "status": "done",
    "deliver": "固定范围反向引用、关系遍历、解释路径与有界查询。",
    "gate": "按当前权限过滤；隐藏桥不扩展；截断和失败明确区分。",
    "evidence": "284 通过 / 3 忽略；已验收",
    "link": "https://github.com/MansurnamedZhang/learning-system-p0c4/blob/addf1193c98e18c6eadb548ed635c35eaff95707/docs/p0b4-verification.md"
  },
  {
    "id": "P0-C1",
    "phase": "可靠性与可移植性",
    "name": "原件、图片与附件",
    "status": "done",
    "deliver": "不可变原字节、资源版本定位、v3 图片/附件及精确使用授权。",
    "gate": "独立卷原字节核验；中断/损坏/回滚/撤权；四套旧程序升级。",
    "evidence": "328 通过 / 3 忽略；已验收",
    "link": "https://github.com/MansurnamedZhang/learning-system-p0c4/blob/addf1193c98e18c6eadb548ed635c35eaff95707/docs/p0c1-verification.md"
  },
  {
    "id": "P0-C2",
    "phase": "可靠性与可移植性",
    "name": "持久任务与受限 Worker",
    "status": "done",
    "deliver": "同事务事件、持久转投、数据库租约、有限重试、资产完整性处理器。",
    "gate": "专用 Worker 真 SIGKILL/接管/取消/撤权；旧 token 拒绝；唯一结果。",
    "evidence": "388 通过 / 0 失败 / 3 忽略；965 份证据哈希一致",
    "link": "https://github.com/MansurnamedZhang/learning-system-p0c4/blob/addf1193c98e18c6eadb548ed635c35eaff95707/docs/p0c2-verification.md"
  },
  {
    "id": "P0-C3",
    "phase": "可靠性与可移植性",
    "name": "融合快照交换",
    "status": "done",
    "deliver": "精确 Reading 快照、版本化目录包与可选原件；导入前校验、原子可见。",
    "gate": "授权闭包往返；缺件/篡改/撤权/同 ID 不同字节拒绝；不出现半可见视图。",
    "evidence": "整关隔离验收与 602 份证据哈希复核通过；未生产部署",
    "link": "https://github.com/MansurnamedZhang/learning-system-p0c4/blob/addf1193c98e18c6eadb548ed635c35eaff95707/docs/p0c3-verification.md"
  },
  {
    "id": "P0-C4",
    "phase": "可靠性与可移植性",
    "name": "完整备份与干净恢复",
    "status": "next",
    "deliver": "受控写入维护窗、数据库一致性备份、资产闭包、异机副本、干净实例恢复。",
    "gate": "固定历史、个人间隙、关系/审查、权限与原件一致；残缺备份不得标 complete。",
    "evidence": "Task5限定收尾；Task6私有16/16；Task7私有6/6；Task8限定完成；Task9与整关OPEN",
    "link": "docs/p0c4-completion-acceptance.md"
  },
  {
    "id": "P1-A",
    "phase": "完整可用产品",
    "name": "登录、API 与内容工作台",
    "status": "planned",
    "deliver": "HTTP 身份/会话、稳定 DTO、连续阅读、块编辑、插入图片笔记与历史冲突处理。",
    "gate": "真实浏览器端到端；跨设备读取服务端保存；冲突保留草稿；HTTP/资产授权。",
    "evidence": "待建设；首次形成浏览器内容工作台",
    "link": "docs/设计批准与实施路线.md"
  },
  {
    "id": "P1-B",
    "phase": "完整可用产品",
    "name": "学习与实践闭环",
    "status": "planned",
    "deliver": "学习单元、计划、任务、尝试、产物、验收、复习；接入首批真实资料。",
    "gate": "今日任务→阅读实践→提交产物→条件化验收；关闭 AI 仍可用；补学习事实恢复。",
    "evidence": "待建设；通过后形成完整学习系统试用版",
    "link": "docs/学习系统设计方案.md"
  },
  {
    "id": "P2",
    "phase": "持续演进",
    "name": "批量演进与多领域复用",
    "status": "planned",
    "deliver": "批量内容升级、迁移候选处理、第二领域、规则推荐与计划调整。",
    "gate": "真实资料更新保留旧事实；第二领域不新增专属核心业务流程。",
    "evidence": "待建设",
    "link": "docs/学习系统设计方案.md"
  },
  {
    "id": "P3",
    "phase": "持续演进",
    "name": "按需求增强",
    "status": "optional",
    "deliver": "AI 辅助、OCR/预览等解析器、语义检索；独立评估 S3 和 Neo4j 可重建投影。",
    "gate": "来源/版本可追溯、权限不绕过、失败可降级；按测量结果决定引入。",
    "evidence": "可选增强；不是首版必装组件",
    "link": "docs/数据库选型实验审阅与首版决策.md"
  }
];

