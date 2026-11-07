> 已被用户要求的 Rust + 前端方案替代，仅保留历史，不作为执行计划。

# P0-A 块修订与权限内核 Implementation Plan

> **For agentic workers:** REQUIRED SUB-SKILL: Use superpowers:executing-plans to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. 在当前任务内顺序执行，不额外创建用户任务；服务器运行沿用已授权的「本地 Linux 服务器」协作。

**Goal:** 在 PostgreSQL 上实现可独立验证的块修订内核，支持历史保留、当前授权、幂等重试、并发冲突与事务回滚。

**Architecture:** 正式应用与现有课程文件分目录。领域契约、业务服务、SQL 和数据库连接分开；块身份与不可变修订单独存储。本交付件由受信任的应用内部调用，不包含公开 HTTP 接口、编辑器或正式部署。

**Tech Stack:** Python 3.12、psycopg 3.3.3、PostgreSQL 18.6、pytest、Linux Docker Compose。前三者沿用隔离实验中已运行的版本基线；实现时把实际依赖解析结果保存为锁定清单，运行日志同时记录 Python、驱动和数据库精确版本。

**Spec:** [批准记录与实施路线](../设计批准与实施路线.md)、[总体设计](../学习系统设计方案.md)、[块更新协议](../内容分块与存储规范.md#9-块级更新协议)、[实验审阅](../数据库选型实验审阅与首版决策.md)。相对路径以本计划目录为基准。

## Global Constraints

以下原句来自获批设计，所有任务共同遵守：

- “内容块是最小编辑和版本单位。”
- “稳定身份与内容修订分开。”
- “个人事实记录与内容分开。”
- “客户端提交目标块、基础修订、修改内容和请求幂等键。”
- “同一幂等键与同一请求内容重试返回原结果；同键不同内容拒绝。”
- “原件独立且不可变。”
- “首版默认单人私有实例。”
- “首版不承诺自动合并或实时协作算法。”

本计划补充明确的实现边界：

- P0-A 只接收 `kind=text` 的严格正文结构；其它形式及跨块引用由 P0-B/P0-C 的类型契约扩展，当前请求必须拒绝而非静默丢字段。
- `intent` 保留 knowledge/note/question/idea/conjecture/observation/evidence/conclusion 八种值；用途不自动赋予任何认识结论。
- 不接收客户端声明的作者或权限；内部 `Principal` 必须由受信任入口建立。正式 HTTP 登录是 P1-A 的独立交付件。
- 权限以查询时的权威空间授权为准；旧修订存在不等于当前仍有读取权。数据库管理员读取能力不计作应用用户能力。
- 不复制实验的裸 `bodies(ids)` helper。批量正文与单条读取执行相同授权。
- P0-A 不宣称解决带 `basis_refs` / `target` 的全部泄露；P0-B 必须在引入这些结构时补齐实验失败场景。
- 只使用新的 `learning-system-p0` 测试项目，2 CPU / 4 GiB 的数据库限额，无数据库宿主端口。不得重置图对比实验或已有业务数据库。
- 本计划中的代码和测试片段是实施目标，当前尚未创建对应应用代码或运行这些测试。

---

## 文件责任与统一接口

所有拟创建文件位于 `仓库根目录`，以下路径相对此目录。

```text
pyproject.toml                         # Python 包及测试入口
requirements.lock                     # 实际安装后锁定的完整依赖
.gitignore                            # 排除凭据、运行数据、缓存与测试产物
modules/__init__.py
modules/content/__init__.py
modules/content/contracts.py           # 请求/响应对象、严格校验与摘要
modules/content/errors.py              # 业务错误，无数据库明细泄露
modules/content/service.py             # 创建/修订事务、幂等和并发控制
modules/content/queries.py             # 当前授权下的单条/批量历史读取
modules/content/sql.py                 # 固定参数化 SQL，不拼接用户输入
modules/identity/__init__.py
modules/identity/principal.py           # 受信任内部主体；此阶段无登录实现
adapters/__init__.py
adapters/database.py                   # 连接工厂、角色和事务边界
migrations/0001_content_core.sql        # 数据、约束、索引和运行角色权限
deploy/compose.test.yaml               # 仅隔离测试 PG 与测试 runner
deploy/Dockerfile.test                 # 运行测试所需环境
tests/conftest.py                      # 独立空间、主体和连接 fixture
tests/unit/test_contracts.py
tests/integration/test_schema.py
tests/integration/test_revisions.py
tests/integration/test_authorization.py
tests/integration/test_concurrency.py
tests/integration/test_atomicity.py
docs/p0a-runbook.md                     # 精确重跑命令与已验证边界
```

不创建其它业务模块空目录。Web、资源、组合、关系和学习模块随相应任务加入。

统一接口在 Task 1 定义，后续任务不自行更名：

```python
from dataclasses import dataclass
from uuid import UUID

@dataclass(frozen=True)
class Principal:
    actor_id: UUID

@dataclass(frozen=True)
class TextDraft:
    title: str
    text: str
    intent: str = "knowledge"
    language: str = "zh-CN"

@dataclass(frozen=True)
class Revision:
    block_id: UUID
    revision_id: UUID
    parent_revision_id: UUID | None
    draft: TextDraft
    content_sha256: str

# 由 contracts.py 提供：
# parse_text_draft(raw: dict) -> TextDraft
# canonical_content(draft: TextDraft) -> bytes
# content_digest(draft: TextDraft) -> str

# 由 service.py 提供；service_factory 为每次调用提供新连接：
# create_block(actor: Principal, space_id: UUID, draft: TextDraft,
#              request_id: UUID, reason: str) -> Revision
# revise_block(actor: Principal, block_id: UUID, base_revision_id: UUID,
#              draft: TextDraft, request_id: UUID, reason: str) -> Revision

# 由 queries.py 提供：
# read_revision(actor: Principal, revision_id: UUID) -> Revision | None
# read_revisions(actor: Principal, revision_ids: list[UUID]) -> list[Revision]
```

`service_factory` 是测试与运行入口提供的构造器，返回绑定连接工厂的 `ContentService`；后续 HTTP 层通过同一服务调用，不自行写 SQL。读取的列表按请求中首次出现顺序返回，去重，遗漏不可见与不存在对象，不返回被过滤数量。

业务错误：`InvalidDraft`、`NotFound`、`RevisionConflict(current_revision_id)`、`IdempotencyConflict`。无权限与不存在在服务边界统一为 `NotFound`；写入冲突只向仍有该块权限的调用者返回当前修订 ID。

## Task 1：严格文本块契约与稳定摘要

**Files:** Create `pyproject.toml`、`modules/identity/principal.py`、`modules/content/contracts.py`、`modules/content/errors.py`、对应 `__init__.py`；Test `tests/unit/test_contracts.py`。

**Interfaces:** Consumes `dict` 请求；Produces 上述 `Principal`、`TextDraft`、`Revision`、业务错误和三个契约函数。

- [ ] **Step 1：先写类型与拒绝行为测试。**

```python
import pytest
from modules.content.contracts import parse_text_draft, content_digest
from modules.content.errors import InvalidDraft

def test_reject_unknown_and_unimplemented_reference_fields():
    raw = {"kind": "text", "intent": "conclusion", "language": "zh-CN",
           "title": "结论", "payload": {"format": "markdown", "text": "条件化结论"}}
    with pytest.raises(InvalidDraft):
        parse_text_draft({**raw, "basis_refs": ["hidden-revision"]})
    with pytest.raises(InvalidDraft):
        parse_text_draft({**raw, "author_id": "client-forged-author"})

def test_equal_content_has_equal_digest_regardless_of_input_key_order():
    raw = {"kind": "text", "intent": "note", "language": "zh-CN",
           "title": "注记", "payload": {"format": "markdown", "text": "保留空格  与换行\n"}}
    assert content_digest(parse_text_draft(raw)) == content_digest(
        parse_text_draft(dict(reversed(list(raw.items())))))
```

- [ ] **Step 2：运行 `python -m pytest tests/unit/test_contracts.py -q`，确认失败由缺失契约实现引起。** 实现时首次准备依赖并记录锁定清单；不要把下载失败当作有意义的测试失败。
- [ ] **Step 3：实现严格字段检查。** 外层只接受 kind/intent/language/title/payload；payload 只接受 format/text，且 format 必须为 markdown。标题与文本必须是字符串，未知 kind/intent/字段拒绝。`TextDraft.__post_init__` 同样校验字段类型和 intent，让直接构造对象的内部调用也不能绕过检查。不 trim 或重写正文、公式样式文本、空格及换行。

摘要只覆盖内容，不覆盖身份、作者、创建时间或修改理由：

```python
import hashlib
import json

def canonical_content(draft: TextDraft) -> bytes:
    value = {"schema_version": "1.0", "kind": "text", "intent": draft.intent,
             "language": draft.language, "title": draft.title,
             "payload": {"format": "markdown", "text": draft.text}}
    return json.dumps(value, ensure_ascii=False, sort_keys=True,
                      separators=(",", ":"), allow_nan=False).encode("utf-8")

def content_digest(draft: TextDraft) -> str:
    return hashlib.sha256(canonical_content(draft)).hexdigest()
```

- [ ] **Step 4：补充改变正文时摘要改变、未知用途拒绝、字符串不能被自动转换、空正文作为草稿可保存的断言，重跑单元测试。**
- [ ] **Step 5：记录本任务验证命令与结果；若源码目录已是 Git 仓库，提交本任务涉及文件。** 不把课程资料加入版本管理。

## Task 2：真实 PostgreSQL 表与不可变约束

**Files:** Create `migrations/0001_content_core.sql`、`adapters/database.py`、`tests/conftest.py`、`deploy/compose.test.yaml`、`deploy/Dockerfile.test`、`.gitignore`；Test `tests/integration/test_schema.py`。

**Interfaces:** Consumes Task 1 契约；Produces 数据库连接工厂和 `db` fixture。`db.admin` 是仅供迁移/断言的管理连接；`db.runtime_connection()` 返回实际非所有者运行角色的新连接；`db.seed_space(actor_id, can_write=True)` 创建测试空间与授权并返回 UUID；`db.revoke(actor_id, space_id)` 提交当前授权撤销。每个测试使用独立 UUID 空间，不清空共享业务数据库。

- [ ] **Step 1：写数据库结构测试，先断言表存在和运行角色不具备超级用户、BYPASSRLS 或表所有权。**

```python
def test_runtime_is_not_database_owner(db):
    with db.runtime_connection() as conn:
        flags = conn.execute("SELECT rolsuper, rolbypassrls FROM pg_roles "
                             "WHERE rolname=current_user").fetchone()
        assert flags == (False, False)
        owner = conn.execute("SELECT tableowner FROM pg_tables "
                             "WHERE schemaname='public' AND tablename='block_revision'").fetchone()[0]
        assert owner != conn.execute("SELECT current_user").fetchone()[0]
```

- [ ] **Step 2：在新测试项目启动 PG 和 runner，先运行 `python -m pytest tests/integration/test_schema.py -q`，记录缺少表/角色的失败。** Compose 使用 PG 18.6，数据库仅内部网络；密码由运行环境提供，不写入源码。记录实际镜像摘要；测试 runner 固定 Python 3.12 与锁定依赖。
- [ ] **Step 3：实现以下表和约束。** UUID 由服务生成；迁移使用独立管理角色；运行角色只能按列更新工作头，不可更新或删除修订。

```sql
CREATE TABLE app_user (id uuid PRIMARY KEY);
CREATE TABLE space (id uuid PRIMARY KEY, owner_id uuid NOT NULL REFERENCES app_user(id));
CREATE TABLE space_grant (
  actor_id uuid REFERENCES app_user(id), space_id uuid REFERENCES space(id),
  can_write boolean NOT NULL DEFAULT false, PRIMARY KEY (actor_id, space_id)
);
CREATE TABLE block (
  id uuid PRIMARY KEY, space_id uuid NOT NULL REFERENCES space(id),
  head_revision_id uuid, UNIQUE(space_id, id)
);
CREATE TABLE block_revision (
  id uuid PRIMARY KEY, space_id uuid NOT NULL, block_id uuid NOT NULL,
  parent_revision_id uuid, content jsonb NOT NULL, content_sha256 text NOT NULL,
  author_id uuid NOT NULL REFERENCES app_user(id), reason text NOT NULL,
  created_at timestamptz NOT NULL DEFAULT clock_timestamp(),
  CHECK (parent_revision_id IS NULL OR parent_revision_id <> id),
  UNIQUE(space_id, block_id, id),
  FOREIGN KEY(space_id, block_id) REFERENCES block(space_id, id),
  FOREIGN KEY(space_id, block_id, parent_revision_id)
    REFERENCES block_revision(space_id, block_id, id)
);
ALTER TABLE block ADD CONSTRAINT valid_head
  FOREIGN KEY(space_id, id, head_revision_id)
  REFERENCES block_revision(space_id, block_id, id) DEFERRABLE INITIALLY DEFERRED;
CREATE TABLE mutation_receipt (
  actor_id uuid NOT NULL REFERENCES app_user(id), request_id uuid NOT NULL,
  request_sha256 text NOT NULL, revision_id uuid NOT NULL REFERENCES block_revision(id),
  PRIMARY KEY(actor_id, request_id)
);
CREATE INDEX revision_block ON block_revision(block_id);
CREATE INDEX block_space ON block(space_id);
```

运行角色拥有读取这些表的数据库权限，但用户授权由服务的参数化查询执行；不能把数据库角色权限当作已完成用户权限。`block_revision`、`mutation_receipt` 仅授予 SELECT/INSERT，`block` 授予 SELECT/INSERT 及 `UPDATE(head_revision_id)`；不授予 TRUNCATE、DROP 或授权表写入。迁移角色负责 fixture 和未来身份模块的管理入口。

- [ ] **Step 4：追加并运行三项结构测试：跨块父修订被外键拒绝；工作头指向另一块被拒绝；运行角色直接 UPDATE/DELETE 修订被拒绝。** 每项使用独立事务，捕获数据库错误后回滚，再继续测试。
- [ ] **Step 5：保存镜像摘要、依赖锁定清单与角色测试结果。** 测试环境以 `docker compose -p learning-system-p0 -f deploy/compose.test.yaml run --rm runner python -m pytest tests/integration/test_schema.py -q` 为唯一集成入口。

## Task 3：创建、修订与幂等事务

**Files:** Create `modules/content/service.py`、`modules/content/sql.py`；Test `tests/integration/test_revisions.py`、`tests/integration/test_atomicity.py`。

**Interfaces:** Consumes 连接工厂、Principal、TextDraft；Produces `ContentService.create_block` / `revise_block`。测试 fixture `service_factory()` 返回此服务，每次方法调用使用独立 autocommit 连接和显式事务。

- [ ] **Step 1：编写保留历史和幂等测试。**

```python
from uuid import uuid4
from modules.content.contracts import TextDraft

def test_retry_returns_original_revision_and_old_content_survives(db, owner, service_factory):
    space_id = db.seed_space(owner.actor_id)
    service = service_factory()
    first = service.create_block(owner, space_id, TextDraft("标题", "旧文"), uuid4(), "初稿")
    request_id = uuid4()
    second = service.revise_block(owner, first.block_id, first.revision_id,
                                  TextDraft("标题", "新版"), request_id, "纠正说明")
    retry = service.revise_block(owner, first.block_id, first.revision_id,
                                 TextDraft("标题", "新版"), request_id, "纠正说明")
    assert retry == second
    assert second.parent_revision_id == first.revision_id
    assert db.admin.execute("SELECT count(*) FROM block_revision WHERE block_id=%s",
                            (first.block_id,)).fetchone()[0] == 2
    assert db.admin.execute("SELECT content->'payload'->>'text' FROM block_revision WHERE id=%s",
                            (first.revision_id,)).fetchone()[0] == "旧文"
```

`owner` 为 `Principal(uuid4())` fixture，并由管理连接插入 app_user；fixture 不向服务传入数据库管理员身份。

- [ ] **Step 2：运行 `tests/integration/test_revisions.py`，确认未实现服务导致失败。**
- [ ] **Step 3：按下列事务顺序实现，禁止先写新修订再在另一事务推进工作头。**

```python
# 每个公共方法使用的新连接均为 autocommit=True。
with connection_factory() as conn:
    with conn.transaction():
        # 1. 按(actor_id, request_id)串行化重复请求；哈希碰撞只多串行，不改变语义。
        conn.execute("SELECT pg_advisory_xact_lock(hashtextextended(%s, 0))",
                     (f"{actor.actor_id}:{request_id}",))
        # 2. 查当前授权；锁授权行 FOR SHARE，撤销授权的写事务与其有明确先后。
        # 3. 查 receipt；相同完整请求摘要返回原修订，不同摘要抛 IdempotencyConflict。
        # 4. 创建时插入 block；修订时 SELECT block FOR UPDATE，锁取得后比较基础版本。
        # 5. INSERT 新 revision；UPDATE block.head_revision_id；INSERT receipt。
        # 6. 退出事务全部提交；任何异常全部回滚。
```

完整请求摘要必须包含操作种类、目标空间/块、基础修订、规范正文与修改理由，不能只用正文摘要。返回已有 receipt 前仍验证当前空间授权；不能利用重试接口取回已撤权的历史正文。缺少写权限时返回 NotFound；基础修订不匹配时返回 RevisionConflict，不创建旁支或新工作头。

- [ ] **Step 4：补测同键不同正文、同键不同目标、只读授权写入、伪造跨块基础修订和响应丢失后的重试，运行该测试文件。**
- [ ] **Step 5：添加一次真实事务故障测试。** 管理 fixture 临时设置仅对本测试空间生效的约束触发器，在 INSERT receipt 时抛错；通过正常服务调用触发。断言新修订与 receipt 不存在、原 head 未改变，最后由 fixture 移除测试触发器。不得把仅模拟 Python 返回值当作数据库回滚证明。

## Task 4：单条与批量读取的当前授权

**Files:** Create `modules/content/queries.py`；Modify `modules/content/sql.py`；Test `tests/integration/test_authorization.py`。

**Interfaces:** Consumes Principal 与固定 revision UUID；Produces `read_revision`、`read_revisions`。测试 `reader_factory()` 使用与 service_factory 相同运行角色连接工厂，返回提供这两个方法的 `ContentReader`。

- [ ] **Step 1：编写当前权限撤销后的读取测试。**

```python
def test_revocation_blocks_single_and_batch_bodies(db, owner, service_factory, reader_factory):
    space_id = db.seed_space(owner.actor_id)
    revision = service_factory().create_block(
        owner, space_id, TextDraft("私有标题", "私有正文"), uuid4(), "创建")
    reader = reader_factory()
    assert reader.read_revision(owner, revision.revision_id) == revision
    db.revoke(owner.actor_id, space_id)
    assert reader.read_revision(owner, revision.revision_id) is None
    assert reader.read_revisions(owner, [revision.revision_id]) == []
```

本文件显式导入 Task 3 示例使用的 `TextDraft` 和 `uuid4`。`db.revoke` 必须通过另一管理连接提交删除 space_grant，不在同一连接中伪造缓存结果。

- [ ] **Step 2：运行该测试文件，确认读取实现缺失或错误时失败。**
- [ ] **Step 3：以一条带权威授权连接的查询取正文。**

```sql
SELECT r.id, r.block_id, r.parent_revision_id, r.content, r.content_sha256
FROM block_revision r
JOIN block b ON (b.id, b.space_id) = (r.block_id, r.space_id)
JOIN space_grant g ON g.space_id = b.space_id
WHERE g.actor_id = %(actor_id)s AND r.id = ANY(%(revision_ids)s);
```

SQL 参数由服务提供，禁止客户端拼接过滤器。单条方法复用批量查询并取首项；规范返回顺序、去重与空列表行为。所有公开业务读取走此边界，数据库内部管理读取不暴露给 API。

- [ ] **Step 4：追加另一个无权主体、混合可见/不可见 ID、重复 ID、随机不存在 ID、空列表和只读授权测试。** 断言响应序列化内容不含隐藏块 ID、修订 ID、标题、正文；不返回漏出隐藏数量的 `total_requested` / `denied_count`。
- [ ] **Step 5：验证修订后的旧版仍可读，撤权后的幂等重试同样被拒绝。** 权限测试成功只覆盖 P0-A 的严格 text 契约；跨块载荷引用的测试进入 P0-B。

## Task 5：两个真实连接的竞争与冲突

**Files:** Test `tests/integration/test_concurrency.py`；Modify `modules/content/service.py` 仅用于修复该测试暴露的事务问题。

**Interfaces:** Consumes Task 3 服务和独立连接工厂；Produces 对“只有一次保存成功”的实际并发证据。

- [ ] **Step 1：写两个请求共享基础修订、各用不同幂等键的竞争测试。**

```python
from concurrent.futures import ThreadPoolExecutor
from threading import Barrier
from uuid import uuid4
from modules.content.contracts import TextDraft
from modules.content.errors import RevisionConflict

def test_two_devices_cannot_silently_overwrite(db, owner, service_factory):
    space_id = db.seed_space(owner.actor_id)
    first = service_factory().create_block(owner, space_id, TextDraft("标题", "基础"), uuid4(), "创建")
    barrier = Barrier(2)
    def edit(text):
        service = service_factory()
        barrier.wait(timeout=10)
        try:
            result = service.revise_block(owner, first.block_id, first.revision_id,
                                          TextDraft("标题", text), uuid4(), "并发保存")
            return ("saved", result.revision_id)
        except RevisionConflict as error:
            return ("conflict", error.current_revision_id)
    with ThreadPoolExecutor(max_workers=2) as pool:
        results = list(pool.map(edit, ["设备甲", "设备乙"]))
    assert sorted(row[0] for row in results) == ["conflict", "saved"]
    assert results[0][1] == results[1][1]
    assert db.admin.execute("SELECT count(*) FROM block_revision WHERE block_id=%s",
                            (first.block_id,)).fetchone()[0] == 2
```

- [ ] **Step 2：运行该测试，确认两个调用实际使用不同 PostgreSQL 连接；将连接 backend PID 写入测试诊断，不输出连接串或密码。**
- [ ] **Step 3：若失败，只修复 Task 3 的锁定和比较顺序。** 不能通过全进程 Python 锁掩盖跨进程事务缺陷。
- [ ] **Step 4：增加相同幂等键的并发请求，断言两者返回同一 revision 且总计只有一个新修订；增加不同块的并发保存，断言两者都成功。**
- [ ] **Step 5：记录检查结果。** 单次竞争与幂等通过只证明这些写路径；组合环、原子发布和位置冲突各有后续测试，不扩大本任务结论。

## Task 6：隔离服务器重跑与交付

**Files:** Create `docs/p0a-runbook.md`；Update `requirements.lock` 和本计划复选框；运行产物放在 Git 忽略的 `artifacts/p0a/`。

**Interfaces:** Consumes 完整源码、迁移、契约和测试；Produces 源码清单 SHA-256、版本信息、测试结果、未运行项和复跑说明。

- [ ] **Step 1：本地执行纯契约测试，确认没有对已有课程文件的写入。** 若本地缺少 Docker，由服务器执行真实 PG 测试；本地不能用 SQLite 替代后宣称 PostgreSQL 通过。
- [ ] **Step 2：向「本地 Linux 服务器」提供完整源码路径、文件摘要和下列检查命令。** 沿用已有用户授权，不另外创建任务；仅使用 `learning-system-p0` 项目。

```bash
docker compose -p learning-system-p0 -f deploy/compose.test.yaml build runner
docker compose -p learning-system-p0 -f deploy/compose.test.yaml up -d --wait db
docker compose -p learning-system-p0 -f deploy/compose.test.yaml run --rm runner \
  python -m pytest tests/unit tests/integration -q --junitxml=artifacts/p0a/junit.xml
docker compose -p learning-system-p0 -f deploy/compose.test.yaml stop db
```

Compose 要定义数据库健康检查、内部 runner 数据库连接和 artifacts 挂载；失败时仍归档日志并停止本测试库，不删除卷。测试 fixture 负责以管理连接执行迁移，再以非所有者运行角色测试业务服务；migration DSN 与 runtime DSN 分开且不写日志。

- [ ] **Step 3：来源任务复核测试总数、失败/跳过项、数据库精确版本、运行角色和文件摘要，检查关键断言确实执行。** 不能仅以 pytest 退出码或回传消息作为全部证据。
- [ ] **Step 4：填写 P0-A 验收记录。** 通过条件：不可变修订、父修订约束、旧版读取、当前授权、批量正文、幂等、并发冲突、事务回滚全部通过；未运行任一关键项则保持未完成。
- [ ] **Step 5：将实际接口用于 P0-B 施工单。** 下一阶段必须包含组合与个人层原子提交、不同稳定块的拆分、`basis_refs`/个人 `target` 的权限投影、关系和路径计数过滤，不能把当前阶段的成功代替这些能力。

## 自检与来源

该计划只实现实施路线中的 P0-A；总体设计其它验收条款已在实施路线第 4 节映射，未被删减。方法名称、返回类型和 fixture 接口按上文统一。正式代码创建前，实施者需同时读取设计规范与本计划。

显式事务使用独立 autocommit 连接，进入 `transaction()` 后一起提交或回滚，避免把外层隐式事务中的 savepoint 误当整次提交。[Psycopg 事务文档](https://www.psycopg.org/psycopg3/docs/basic/transactions.html)

并发实现依赖 PostgreSQL 的行锁与事务级 advisory lock；锁顺序和事务结束行为用真实连接验证。[PostgreSQL 18 锁文档](https://www.postgresql.org/docs/18/explicit-locking.html)

fixture 用于隔离测试数据和确保异常路径后的清理；跨线程竞争必须使用独立连接。[pytest fixture 文档](https://docs.pytest.org/en/stable/how-to/fixtures.html)
