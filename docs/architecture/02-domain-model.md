# 02 块与版本化领域模型

## 一切皆块的准确含义

可以独立阅读、修订、复用的知识、笔记、图片、想法、猜想与证据正文采用统一内容块体系。身份、权限、出现位置、任务、资产字节、回执和学习事实仍有独立模型；不是把所有数据库行强行塞成同一种块。

至少区分三种粒度：Block 是长期内容维护单位；LearningUnit 是未来学习目标、任务与验收组织单位；RetrievalChunk 是可重新切分的检索派生片段。后两者不能替代块的长期身份。

![身份、版本与位置关系](diagrams/knowweave-domain.svg)

## 主要实体

| 实体 | 职责 | 版本或引用规则 |
| --- | --- | --- |
| Block / BlockRevision | 稳定内容身份与不可变正文 | 改正文生成 revision；相同正文摘要不合并业务身份 |
| Composition / CompositionRevision | 组织有序块或嵌套组合 | 成员固定版本，嵌套组合不得成环 |
| Occurrence | 组合中某一次出现 | 同一块重复使用时分别定位；保留完整出现路径 |
| Release | 固定发布根与可复现依赖闭包 | 校验与发布原子提交；不等同于可变工作头 |
| Overlay / OverlayRevision | 属于个人的插入与编排层 | 固定原文基线，不直接修改原文 |
| Placement / GapAnchor | 个人块在原文间隙的位置 | 正文与移动分别修订；不依赖屏幕坐标定位 |
| ReadingView / ReadingViewRevision | 原文、个人层与所选关系的阅读版本 | 支持 Original、Fused、Personal 投影 |
| Relation / RelationRevision | 有类型、有方向的语义关系 | 端点固定具体块修订，关系自身也有修订 |
| RelationReview / EpistemicReview | 关系审查和认识判断 | 固定依据及适用条件，不由关系数量自动推断真伪 |
| Lineage | 派生、拆分与合并谱系 | 保存前后身份与引用，不冒用旧 ID |
| Asset / ResourceVersion / SourceSegment | 文件身份、逻辑资源版本与来源定位 | 资产字节不可变；业务资产身份不同于内容摘要 |

数据库实体与 Rust 类型不是一对一映射。命令、查询投影、回执和精确引用也都是类型，但不应在架构图中画成独立服务。

## 正文形式与用途

`intent` 表示 knowledge、note、question、idea、conjecture、observation、evidence、conclusion。形式与用途分别表达；一段图片说明可以服务于笔记或证据，不需要制造另一套笔记文件系统。

当前已实现正文合同：

| 合同 | 已支持形式 | 尚不能推断的能力 |
| --- | --- | --- |
| v1 | Markdown 文本与八类用途 | 不是整个系统的交换包 1.0 |
| v2 | 文本、精确块引用、关系选择视图 | 不是任意自由 JSON 或任意对象 target |
| v3 | 在版本化正文体系中加入 Figure、Attachment | 不等于独立公式、代码、表格、练习、评分规则类型全部实现 |

公式和代码可以作为文本内容展示；若需要结构化编辑、类型校验和专门执行语义，应追加自己的合同与验收，不能仅凭 Markdown 能显示就宣称已支持该业务类型。

## 身份与摘要

Block ID 代表“哪一个知识对象”，Revision ID 代表“这个对象的哪一版”，正文摘要用于核对规范内容字节。两个来源或归属不同的块即使正文相同也可拥有不同身份。

精确引用携带稳定 ID 与修订 ID。关系、关系审查和认识审查拥有各自的精确引用类型；依赖还标明 basis、target、requires_context、source_run、selected_relation、selected_review 等作用。禁止将未知字段或未登记引用藏在自由载荷中绕过授权。

可变 head 便于继续编辑；正式发布与阅读修订固定依赖，不能自动换成 head。正文、位置、关系、包格式和数据库迁移拥有不同版本维度，不能用一个“项目 v2”替代这些规则。

## 连续阅读与插入

```text
原文 A @固定修订
  个人笔记 N @修订1
  个人图片 I @修订2
  猜想 H @修订1
原文 B @固定修订
  引用另一文档中的块 R @固定修订
原文 C @固定修订
```

间隙锚点固定原文组合修订、完整 occurrence 路径、左右邻居及附着偏好。文首、文尾和两块之间均可定位。页面的“连在一起”是阅读投影；不把个人块直接写入别人的源组合。

原文更新后保留旧基线，计算可迁移候选。映射不能确定时保留待重新放置项，用户仍可找回内容。自动保存、任意间隙插入控件、外观区分和迁移确认页面属于未来前端；当前已有对应阅读和位置内核。

## 猜想与知识修订的两条主线

“猜想—证据—反例—结论”通过有方向关系和条件化审查表达。猜想修订后，不自动继承旧版结论；证据撤权后，当前投影不泄露隐藏标识和路径。

“知识修订—影响范围”先创建新修订，再找出固定范围内的引用者、具体出现位置、关系及解释路径。结果用于选择采用新版的组合，不能自动改写已发布文档或历史事实。

支持/反对的知识判断已实现；“某学习任务是否通过、能力是否掌握”是 P1-B 待建设的另一套条件化验收，不可混淆。

## 实现依据

契约见 [content.rs](../../crates/learning-core/src/content.rs)、[content_version.rs](../../crates/learning-core/src/content_version.rs)、[references.rs](../../crates/learning-core/src/references.rs)、[overlay.rs](../../crates/learning-core/src/overlay.rs)、[relation.rs](../../crates/learning-core/src/relation.rs)。模式见 migrations（私有或未公开参考）；实际限制以合同类型与数据库约束为准。
