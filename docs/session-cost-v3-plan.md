# Sessions Cost v3 实施计划（已定稿，待实现）

> 状态：**设计已收敛并经三轮外部评审，产品决策已由用户拍板，可进入实现。**
>
> 定稿日期：2026-07-31
>
> 本文取代 `session-cost-performance-handoff.md` 中的"推荐恢复方向"（§8）。
> 旧文档仍是背景与现状基线的权威记录；两者冲突处**以本文为准**。
> 被本文明确取代的旧约束有两条：
> 1. "数据不完整只能显示 `-`" → 改为 `≥` 分级展示（用户决策 ①）；
> 2. "Sessions 不触发 Usage sync" → 手动刷新时允许一次性增量 sync（用户决策 ④）。

## 0. 一句话目标

**有没有 Cost 列，Sessions 页的打开/翻页/刷新速度都必须在同一量级。**
实现手段：Cost 永远是"当前页（≤100 行）的异步只读投影"；全量 Cost 索引
（当前未提交实现中的 Phase B / `session_metrics::index_manifest`）整体删除。

## 1. 已锁定的产品决策（用户拍板，不得改动）

1. **不完整费用显示 `≥$1.23`**，Hermes 显示 `~$1.23`（估算），完全无数据显示 `-`。
2. **不做周期性自动补数字**：60 秒周期 sync 完成后不自动重查费用；费用只在
   进页、翻页、搜索、locate、手动刷新时更新。
3. **做"可证明完整"档（Complete）**，前提是**不改主数据库 schema**——本设计
   满足：只向既有 `settings` KV 表写一个 key，以及在 manifest（磁盘 JSON）中
   加一个 Option 字段；不建表、不加列、不 bump schema version。
4. **手动刷新顺带同步**：按 `r` 后列表照常秒出（metadata 发布不被阻塞）；后台
   触发一次增量 usage sync（实测 ~1.7s）；sync 到达任意终态后（Ok/Err 都可能
   已改库）对当前可见页自动重发一次 Cost 查询。一次性行为，无周期链路。

## 2. 语义模型

### 2.1 数值定义（精确措辞，写进 `?` 帮助）

显示值 = **当前 `proxy_request_logs` 中，经 effective filter 去重后，能通过
确定性 ID 映射归属到该 session、且仍保留的明细小计**（retained attributable
detail subtotal）。它不是端到端账单，也不是"历史上曾记录过的全部金额"。

### 2.2 数据结构

`SessionUsageSummary`（`session_manager/mod.rs`）重构为：

- token 四桶（input / output / cache_read / cache_creation）
- `cost: Option<f64>`（不得用 0 / NaN / 负数当哨兵）
- `cost_kind: Recorded | Estimated`
- `coverage: Complete | Partial | Unknown`

`SessionMeta.usage` 保持 `#[serde(skip_serializing)]` runtime-only，不写入
manifest。

### 2.3 渲染规则

| 条件 | Cost 显示 | Tokens 显示 |
| --- | --- | --- |
| coverage=Complete | `$1.23` | 精确值 |
| Partial/Unknown 且存在已计价的 token-bearing 归属行 | `≥$1.23` | 带同样的覆盖标记（不得裸显示成精确值） |
| Hermes（源字段本名 `estimated_cost_usd`） | `~$1.23` | 同上 |
| 无 token-bearing 归属行 / 全部行未计价 / ID 歧义 / 查询失败 | `-` | 有行则可显示，无则 `-` |

- **unpriced 判定必须逐行**：某行"正 token ∧（`pricing_model=''` ∨ 行总
  cost=0 且无法证明真零价模型）"⇒ 该 session 封顶 Partial（仍可显示 `≥`，
  因为小计仍是合法下界）；若不存在任何已计价行 ⇒ Cost 为 `-`。
  零 token 的错误行（如 `pricing_model=''` 的失败请求）**不得**毒化 session。
- **重复身份歧义**：manifest 身份是 `(provider_id, session_id, source_path)`
  （`paged_manifest.rs:2435`），Usage DB 只有 `(app_type, session_id)`。同一
  session_id 出现多个不同 source_path 的可见行时，所有这些行显示 `-`，
  不得默默把同一小计展示成每个文件各自的费用。
- Complete 的帮助文案严格限定为"当前 session ID 的已记录小计完整"，不得
  暗示端到端账单；**必须写明 Codex 根会话费用不含独立 subagent 线程的费用**
  （见 §4.3）。

## 3. 运行时架构（异步只读投影）

### 3.1 消息协议

删除同步 `enrich_rows` 语义（实测第一页命中 48,064 行明细、聚合 ~200ms，
不允许出现在页加载路径上）。改为：

```text
页加载完成 → 立即发送 PageLoaded（usage 全为 None，UI 先显示 -）
           → 发送 CostOverlayRequest {
                 cost_seq,                    // 独立自增序号
                 page_token,                  // 完整 SessionPageToken（app/types.rs:944：
                                              //   scope_epoch / view_epoch / source / scope / generation）
                 page_index,
                 row_identities,              // (provider_id, session_id, source_path) 列表
             }
Cost worker → CostOverlayResult {
                 cost_seq, page_token, page_index,
                 overlays: identity → SessionUsageSummary 映射   // 不按数组位置回填
             }
handler 校验：cost_seq == active_cost_seq ∧ page_token 逐字段一致
             ∧ page 仍可见 ∧ 逐行 identity 仍匹配，全过才回填。
`scan_seq` 不参与 Cost 协议。
```

请求触发点 = 原 5 个 overlay 调用位置（页加载 / 搜索首页 / 缓存打开首页 /
重建后首页 / locate 结果页，`workers.rs:1212/1915/1970/2090/2143` 附近），
外加手动刷新的 sync 终态一次性重发（§5）。

### 3.2 Cost worker（单槽 latest-wins）

- 独立单线程 worker，收任务时 `recv_latest` 合并积压请求；
- 共享 atomic `active_cost_seq`：发送新请求即更新；SQLite progress handler
  内检查 `active_cost_seq != my_seq || now >= deadline` 即中断——**仅靠
  recv_latest 不够，必须能打断正在执行的旧 SQL**（否则快速翻页时新页要等
  旧页最多 2 秒）；
- 执行 deadline 2s（正常 ~200ms 的 10 倍护栏）；busy_timeout 250ms（只管
  等锁，不管执行时长）；
- 主库连接用现成 `Database::open_readonly_current_schema()`
  （`database/mod.rs:738`——不建目录、不迁移、不 seed、不跑启动维护，并自带
  future-schema 拒绝），**严禁走 `Database::init()`**（init 会触发
  maintenance/prune，是写路径）；
- 聚合 + 水位 + 完整性判定的所有 SELECT 在**同一个只读事务快照**内完成，
  事务保持短促（查完即结束，不跨请求持有）。

### 3.3 主库查询（Claude / Codex / Gemini / OpenCode）

- 单条 CTE：`WITH wanted(app_type, session_id) AS (VALUES ...) ... GROUP BY
  app_type, session_id`，四 provider 一次查询、共享同一快照；
- 复用既有 SQL 片段：`effective_usage_log_filter("l")`（usage_stats.rs:228）、
  `fresh_input_sql("l")`（sql_helpers.rs:63）、Usage 页的 token 桶与
  `SUM(CAST(total_cost_usd AS REAL))` 语义——与 Usage 页一致性靠共享代码
  保证，不新写计费规则；
- **Codex 双 ID 合并**：manifest ID `U` 同时查 `U` 与 `codex_U`
  （代理侧前缀见 `proxy/session.rs:68,83`）并合并到 `U`；Generated 随机 ID
  无法映射，天然不属于该小计；
- 实现后必须跑 `EXPLAIN QUERY PLAN` 确认仍由 `idx_request_logs_session`
  驱动（改成 VALUES+JOIN 后不得退化为扫日志表再连接），并重跑 top-100
  基准（当前基线：48,064 行 / ~200ms）。

### 3.4 其他 provider

- **Hermes**：直接只读其 `state.db` 聚合（沿用 `session_metrics/hermes.rs`
  的 schema 探测逻辑，但必须把 `IN (wanted)` 下推进 SQL——现实现是全库聚合后
  Rust 侧筛选，`hermes.rs:71-77`，不可原样移植）；busy_timeout 降到 ~250ms；
  一律 `cost_kind=Estimated`、渲染 `~`；不需要门 B。
- **OpenCode**：费用一律来自主库 CTE（**不得**从 OpenCode 自身 DB 重新计价，
  那是第二套规则）；OpenCode 自身域仅提供新鲜度证据（§4.2）；注意 sync key
  规范化（`session_log_sync` 键为 `"{db_path}:{session_id}"`，与 manifest
  路径形式不同）。
- **OpenClaw**：v1 显示 `-`（主 Usage 后端不导入它；解析正文正是本方案要
  消灭的工作类型）。
- 任何失败（DB 不存在 / busy / future schema / 表缺失）→ `log::debug!` +
  该行 None → `-`。整条链路零写入。

## 4. 覆盖分级（coverage 判定）

判定是"分级器"不是"扣数字的门"：任一 Complete 条件不满足只降级为
Partial/Unknown（显示 `≥`），数值本身照常显示。

### 4.1 Complete 的三个必要条件

1. **剪除证据**：`session.created_at`（毫秒，换算后）≥ prune 高水位
   （§6.2 的 settings key）。水位缺失、损坏、或 `history_complete=false`
   ⇒ 封顶 Partial。**不做** `MAX(usage_daily_rollups.date)` 回退——它丢时区、
   有空桶、且 Codex rebuild 正常路径会删 rollup 行
   （`reset_codex_usage_on_conn`，session_usage_codex.rs:297 起）。
2. **created_at 来源可信**：manifest 需新增来源标记
   `created_at_kind: ProviderTimestamp | FileMtimeFallback`。当前 Claude/Codex
   在 payload 缺时间戳时会拿文件 mtime 兜底（`file_modified_ms`，
   claude.rs ~440 / codex.rs ~1273），被 touch/复制过的老文件可借此伪造
   "在保留窗口内"。FileMtimeFallback ⇒ 封顶 Partial。
3. **快照证据**：`session_log_sync.last_modified`（importer 读前采集的
   mtime_ns）≥ `manifest.source_mtime_ns`（§6.1 的新字段）。同域比较。
   老 manifest 无该字段 ⇒ Unknown。
   **禁止**任何"payload 时间戳 vs 文件 mtime/同步墙钟"的跨域比较（评审已给出
   毫秒截断、无时间戳 usage 行回退 now()、旧内容回写、pending-tail 无 sidecar
   等四个确定性反例）。

### 4.2 各 provider 封顶表

| Provider | Complete 可达性 | 原因 |
| --- | --- | --- |
| Codex | **可达**（三条件齐 + 双 ID 合并） | subagent usage 记在子 rollout 自己的 thread ID 下（session_usage_codex.rs:384/2000，测试 :2546/:2904），子文件不进 manifest（codex.rs:1178），根 session 无家族缺口 |
| Claude | 封顶 Partial | 根文件证据不覆盖 `subagents/*.jsonl` 家族；已导入的 subagent 行照常计入小计 |
| Gemini | 可达（同三条件） | 插入失败推进水位、malformed skip 属于数据源自身正确性上限（§9），不参与分级 |
| OpenCode | 封顶 Partial | `MAX(time_updated)` 毫秒级，同毫秒新增消息不可辨；除非引入复合版本（time_updated + 行数/max rowid），否则不升 Complete |
| Hermes | 不适用 | 恒为 Estimated（`~`） |
| OpenClaw | 不适用 | 恒为 `-` |

### 4.3 必须测试钉死的语义

- Codex：child 用自己 ID 入库、parent replay 被剔除、child 不进 manifest、
  **父 session 的 Complete 不受更新的 child rollout 影响**、`U`/`codex_U`
  合并、重复 source_path 歧义 → `-`。
- Claude：subagent 行已导入时计入小计；根文件证据不参与升 Complete。

## 5. 手动刷新流程（决策 ④ 的实现形状）

```text
按 r（force=true）
  → Phase A 元数据重建（既有 head/tail 有界读；本次新增：把 source_mtime_ns
    与 created_at_kind 写入行，复用 cache 层已有的扫描前 stat 与解析后 restat
    比较（cache.rs:622 附近），前后不一致则该行 source_mtime_ns=None；
    不得新增额外 stat 轮次）
  → ManifestPublished（终态；metadata 发布永远不等任何 Cost/sync 工作）
  → 立即对第一页发 CostOverlayRequest（显示当前库里已有的数字）
  → 同时向既有 usage sync worker 队列投递一次增量 sync 请求
    （复用现有单线程 worker 与请求合并机制，不新建线程/定时器）
  → sync 终态消息（Ok 或 Err 都可能已部分提交，SessionUsageSyncMsg::Finished
    只有 Result<(),String>，workers.rs:2558 会把部分成功折叠成 Err）
  → 若 Sessions 页仍可见：对当前可见页重发一次 CostOverlayRequest（仅一次）
```

60 秒周期 sync 的终态**不**触发重查（决策 ②）。

## 6. 写入侧的两处小改动（全部不改主库 schema）

### 6.1 manifest 新增字段（磁盘 JSON，非数据库）

- `SessionMeta` 增加 `source_mtime_ns: Option<i64>` 与
  `created_at_kind: Option<...>`，依赖既有 `#[serde(default)]` 向后兼容；
- **禁止 bump manifest format_version**：bump 会把旧 manifest 判无效并在普通
  进入 Sessions 时触发自动全量重建（workers.rs:1962 的 bootstrap 路径），
  直接违反"进页不扫源"的硬约束。老 manifest 字段为 None → 封顶
  Unknown/Partial，直到用户手动刷新自然补齐。

### 6.2 prune 高水位 settings key

- key 形如 `usage_prune_high_watermark`，值含 `{ epoch, history_complete }`；
- 写入位置：`rollup_and_prune` 的同一 SAVEPOINT 内（usage_rollup.rs:79 附近），
  max-单调更新，**直接用当前持有的 connection 写**，禁止调用会再取
  Database mutex 的公开 setter（死锁）；
- 读取：缺失/损坏/无法解析 ⇒ 视为"无证据"，封顶 Partial，**绝不当 0 处理**；
- 旧库迁移：升级后首次看到无该 key 的库 ⇒ 写入 `history_complete=false`
  并永久封顶 Partial（旧库可能发生过未记录的 prune，不得在第一次新 prune
  后误升级为"历史完整"）；全新建库 ⇒ `history_complete=true`；
- **WebDAV**：该 key 必须加入本地保留白名单 `SYNC_LOCAL_SETTINGS_KEYS`
  （backup.rs，现仅 `proxy_runtime_session`）。usage 明细与 rollup 本就是
  本地保留对象；若水位被远端 restore 覆盖而日志保留本地，证据域会撕裂。
- 需补测试：SAVEPOINT 回滚时水位不前进；旧库迁移；WebDAV restore 后保留。

## 7. 删除清单 / 保留清单

### 删除（相对当前 dirty worktree）

- `services/session_metrics/` 整目录（先把 hermes.rs 的只读聚合逻辑按 §3.4
  改造移植，openclaw.rs 一并删除）；
- `index_manifest` 及手动刷新后的整个 Phase B 调用链（workers.rs:2099-2124）；
- `MetricsProgress`/`MetricsFinished` 全套：runtime_systems/types.rs:212-223、
  workers.rs 两处发送（:2108/:2119）、handlers.rs 两个分支（:593-617）、
  app/types.rs 四个字段两个方法与 reset（:1166-1169/:3612-3670/:3596-3599）、
  ui/sessions.rs 两处（:43/:51-52）、i18n.rs 两条（:9425/:9844）、相关测试；
- 为 scoped import 加的 5 个 `sync_*_sources`/`sync_opencode_session_ids`
  入口及其私有重构（session_usage.rs / _codex / _gemini / _opencode，
  合计约 +889/−87）；
- `Database::derived_cache_at`（database/mod.rs:800 附近）；
- `scan_jsonl_incremental` 的 `is_cancelled` 第 7 参回退（含其测试）。
- 逐项回退必须用 diff 核对，不得覆盖分支基线中与本任务无关的改动。

### 保留

- 55/45 布局、Cost 列、Overview token/cost 行、共用 token formatter；
- `SessionMeta.usage` runtime-only（skip_serializing）；
- 有效 manifest 固定成本打开（无源 revalidation）；metadata-first 发布；
- 5 个 overlay 触发位置（改为异步请求）；
- 磁盘上用户已生成的 `session-metrics-cache-v1.db` / `session-metrics-resume-v1.db`
  是用户数据：代码不再打开即可，**不得删除文件**。

## 8. 性能要求与回退防线（硬性验收）

性能是本任务的核心关注点，分两面：新功能自身要快，且不得拖累任何既有功能。

### 8.1 新功能自身

| 路径 | 要求 |
| --- | --- |
| 有效 manifest 普通进入 | 只读 1 个 page 文件 + 异步发一次 Cost 请求；PageLoaded 不等 Cost；无源目录 walk/stat（用测试断言代码路径） |
| 翻页 / 搜索 / locate | 同上，每次至多一个在途 Cost 查询（latest-wins 吞并旧请求） |
| Cost 查询 | 后台 ~200ms 级（top-100 基准复测）；2s deadline；可被新请求即时打断 |
| 手动刷新 | metadata 发布时间与"无 Cost 版本"相同（Phase A 不新增 stat 轮次）；后台增量 sync ~1.7s 级；终态后一次重查；全程不再出现 minutes 级 `Indexing cost` |

### 8.2 不得拖累别的功能（逐项防线）

1. **主库争用**：overlay 只读事务必须短促（单批查询即结束）；不长期持有
   snapshot（长读事务会推迟 WAL checkpoint、放大 WAL）；busy 250ms 超时即
   优雅 `-`，不重试风暴。与 60s sync 写事务、proxy usage logger 并存时不得
   造成写侧可感知延迟。
2. **usage sync worker**：刷新触发的一次性 sync 走既有队列与合并机制，
   与周期 sync 自然去重；不得新增线程、定时器或在 TUI 启动/进页时加同步工作。
3. **UI 线程**：渲染与按键路径零 SQL、零文件 IO（参照
   docs/tui-blocking-performance-risks.md 的既有纪律）。
4. **proxy 热路径**：零改动。水位写入只在 prune 路径（Database::init 与 24h
   周期）内多一次 KV upsert，不得出现任何 per-request 写入。
5. **Phase A**：source_mtime_ns 必须复用既有的前后 stat，新增 syscall 数为零。
6. **manifest 体积**：新字段对 8MiB page 上限、64KiB 行上限的影响须经测试
   确认可忽略。
7. **启动路径**：AppState/TUI 启动零新增工作；删除 Phase B 后总体是净改善
   （不再构建 53.7MB 派生库）。
8. **回归测量**：改动前后各测一次并记录进 PR：进入 Sessions 页耗时、翻页
   耗时、手动刷新 metadata 发布耗时、Usage 页聚合耗时（应无变化）、
  `cargo test` 目标测试时长（应无量级变化）。按普通用户机器假设评估
  （HDD / 杀软 / 低核数），不得只以本机 NVMe 结论为准。

## 9. 残留上限（威胁模型，写进文档与 `?` 帮助，不阻塞实现）

Gemini 插入失败仍推进水位（session_usage_gemini.rs:320-338）、Claude/Codex
malformed 行跳过后推进游标、代理 Generated ID 无法归属、同 mtime 内容替换、
proxy 实时日志无即时通知、SQLite REAL 求和精度。这些是 Usage 数据源自身的
正确性上限：即使 coverage=Complete 也只是**投影层完整**，不是账单级完整。
上游同样存在的问题不在本任务扩大修复。

## 10. 实现顺序

1. 核对 `git status`、用户进程（不得 kill 用户启动的 cc-switch）、
   `origin/main`（本分支落后 2 个提交：ea296a0、e92c9cc）。
2. 先写测试：coverage 分级真值表（含 §4.3 全部钉死项）、水位写入三测试
   （回滚/迁移/WebDAV）、异步协议过期校验（cost_seq / token / identity /
   页切换）、DB 缺失/busy/future-schema 降级、投影与 Usage 页对账、
   unpriced 逐行判定、无写入断言。
3. 实现 §3 异步链路与 §6 两处写入；随后按 §7 清单删除。
4. `EXPLAIN QUERY PLAN` + top-100 基准 + §8.2.8 回归测量。
5. 隔离目录跑 `cargo fmt --check` / `cargo clippy` / 目标测试；对照基线已知
   失败（§11），不混入无关修复。
6. 合并 `origin/main` 两个提交，逐文件处理冲突（settings/TUI 相关优先细看）。
7. 按仓库 CLAUDE.md / AGENTS.md 的盲审协议：两名全新独立盲审 → 逐条实证 →
   修复 → 新一轮，收敛前不 commit / push / PR。

## 11. 基线已知问题（不要顺手修）

- `cargo clippy` 在 home_chart.rs 既有代码上触发 `reversed_empty_ranges`；
- 集成目标 `settings_current_provider` / `settings_visible_apps` 在基线 HEAD
  即编译失败；
- 均与本任务无关，不得混入本 PR。

## 12. 禁区（沿用 handoff §14，全文有效）

不改主机 `$CC_SWITCH_CONFIG_DIR` / `$CLAUDE_CONFIG_DIR` / `$CODEX_HOME`；
写入型测试一律隔离 home/temp dir；诊断读真实历史保持只读；不删用户 sidecar；
不改主库 schema / rollup / 去重 / 定价语义（§6.2 的 KV 与 SAVEPOINT 内附带
写入是唯一被批准的例外，且不改变 rollup 输入输出语义）；Sessions 无周期
自动刷新；所有 Cargo 命令在 `src-tauri/` 下执行。
