# model_catalog_json 说明

> 面向维护 codex-ui「模型配置 → 生成模型目录」的开发者。codex 官方没有该配置的文档，
> 本文件是目前唯一成文的字段与生成规则说明；权威定义来自 codex 源码的 `ModelInfo`
> 与内置目录 `codex-rs/models-manager/models.json`。

## 它是什么

`CODEX_HOME/config.toml` 里的 `model_catalog_json` 指向一个 JSON 文件，形如：

```json
{ "models": [ { "slug": "deepseek-flash", "...": "..." } ] }
```

- **整份替换**：codex 0.149.0 的注释是 “When set, this replaces the bundled catalog for
  the current process.”，即一旦配置该键，codex 自带的 8 个条目（`gpt-5.6-sol/terra/luna`、
  `gpt-5.5`、`gpt-5.4`、`gpt-5.4-mini`、`gpt-5.2`、`codex-auto-review`）在该进程里全部消失。
  因此要用哪些模型，就得把它们都写进目录（包括想用的 GPT 模型）。
- **只在启动时构建**：改完目录或提供方后需要重启 codex-ui 才生效（`config/batchWrite` 的
  `reloadUserConfig` 不会重建模型目录）。
- **必需字段缺失会导致加载失败**：生成器先补齐键，再校验字段类型与关联约束；
  无法安全生成的单个模型标为 `invalid`，不让它破坏其余模型的目录。

## 生成管线

```
提供方 /models（模型 ID 列表）
   │
   ├─ 1. 完整条目源（official）：本机 codex 导出条目 + 内置第三方条目，命中即整条复用，不套模板
   │
   └─ 2. 字段源：models.dev + OpenRouter
          │  每个源返回匹配类型、分数与 ModelFacts
          │  merge 先比较匹配准确度，再比较来源权威性，逐字段选择
          ▼
      3. 模板渲染（model_catalog_template.json，固定值唯一事实来源）
          │  自动派生、修正约束并校验，失败条目隔离
          ▼
      4. 选择模型 → 确定 → 直接回填完整参数（不自动保存）
```

代码结构（`src-tauri/src/codex/model_catalog/`）：

| 文件 | 职责 |
| --- | --- |
| `mod.rs` | 管线编排、命令入口、`refresh_source_caches`、`ModelCatalogGenerateResult` |
| `facts.rs` | `ModelFacts`（全字段可选）+ `merge_from` 逐字段覆盖 + `provenance` 溯源 |
| `matching.rs` | 别名键匹配、匹配级别与排序键、`CandidateStore`、规范化/相似度 |
| `template.rs` | 模板加载、`DATA_DRIVEN_KEYS`、四条派生规则 |
| `validation.rs` | 关联事实修正、输出结构与上下文校验、最终字段来源补全 |
| `sources/official.rs` | 完整条目源：官方条目池（本机 codex 导出 + 内置第三方条目，精确匹配） |
| `codex_models.rs` | 启动时从本机 codex 导出官方条目（`codex debug models --bundled` → 运行时缓存） |
| `sources/models_dev.rs` | 字段源（catalog.json 全量 provider 展开的模型 ID 索引） |
| `sources/openrouter.rs` | 字段源（缓存 + 内置回退） |

### 新增一个数据源（三步）

1. 实现 trait：`FactSource`（给部分字段）或 `FullEntrySource`（给完整条目）；
2. 在 `sources/mod.rs` 的 `fact_sources()` / `full_entry_sources()` 里按优先级追加一行；
3. 加映射单测 + 在 `scripts/update-model-catalog-sources.mjs` 里补抓取/精简逻辑（若需要内置快照）。

合并、匹配、渲染逻辑都不需要改动。

## 匹配与排序规则

- **权威别名**：OpenRouter 用 `canonical_slug` / `alias_target.slug`；
  models.dev 的 `family` 只参与模糊系列继承，不构成同一模型的精确别名。
- **匹配级别**：规范模型 ID 精确 → 权威别名精确 → 规范化精确 → 同系列模糊匹配 → 无匹配。
  一般模糊阈值为 0.72；版本系列已确认一致时可降到 0.60，容纳 `coder` / `flash` / `pro`
  等规格后缀造成的额外距离。
  规范化会剥掉 provider 前缀、`:free` / `:batch` / `-free` / `-latest` / `-preview` /
  `-beta` / `-alpha` 后缀与结尾日期；日期剥离使用 UTF-8 安全切片，格式为
  `YYYYMMDD` 或 `YYYY-MM-DD`。明确存在且冲突的命名空间不会跨厂商匹配。
- **模糊排序**（依次比较，全部确定性）：同厂商 → 同版本系列 → 保留查询中的规格 token
  （`coder` / `chat` / `reasoner` / `flash` / `pro` / `mini` / `sonnet` 等）→
  正式模型优先于实验/beta/deprecated → 版本距离近者（同系列标记后的首个整数，
  如查询 `deepseek-v5` 时 `deepseek-v4-*` 优于中转商的历史 `deepseek-v3*`）→
  多余 token 少者 → 相似度 → 发布时间新者 → 模型 ID。
- **效果**：`gpt-5.7` 继承最新正式 GPT-5.x，`qwen4-coder` 优先继承 Qwen Coder，
  `deepseek-v5` 只在 DeepSeek V 系列内选择；模糊命中仍是正常可生成模型，只在弹窗显示
  “资料继承自……”的溯源。
- **输出 slug 始终是提供方返回的原始 ID**，只有能力字段来自命中的规范模型。
- **只按模型 ID 匹配**（models.dev）：不按 `base_url` 定位目标提供方，同一个模型 ID
  在任何目标提供方下结果一致。保留 `providers` 全部分组与 `models` 模型级记录，
  不在解析阶段按字典序丢弃同 ID 资料；源内部逐字段采用「规范模型 → 明确原厂 → 其他提供方」。
  原厂仅按规范 ID 命名空间与 provider ID 的明确关系识别，无厂商白名单、无名称猜测。
  无前缀 ID 仅通过唯一、无损尾段关联规范模型，保留日期、版本与规格后缀。
  规范资料缺失的字段仍可补充；其他提供方同级值一致才采用，冲突字段保持未知，
  自动交给其他来源或模板。`big-pickle` 等分组专有 ID 仍可匹配。
- **字段覆盖**：先比较 exact / alias / normalized / fuzzy，再比较来源权威性
  （同级默认 models.dev 优先于 OpenRouter），模糊命中再比较相似度，最后按稳定来源标识排序。
  因此 OpenRouter 的精确资料会覆盖 models.dev 的近似资料；缺失值不抹掉已有结果。

## 字段来源

生成器写入的字段有三类来源：**数据驱动**（字段源）、**派生**（渲染规则）、**固定值**（模板）。

### 两个字段源各自提取什么

两个源产出的是同一套 `ModelFacts`（`facts.rs`，12 个可合并字段），差别只在从上游 JSON 的哪个路径取值：

| ModelFacts 字段 | OpenRouter 取值 | models.dev 取值 | 落到生成条目 / 用途 |
| --- | --- | --- | --- |
| `context_window` | `context_length`（>0） | `limit.context`（>0） | `context_window` |
| `max_context_window` | 同 `context_length`（OpenRouter 没有独立的 max 字段） | 同 `limit.context` | `max_context_window` |
| `input_token_limit` | —（不提供） | `limit.input`（需 >0 且 < context） | 仅用于派生：`effective_context_window_percent = min(95, floor(input/context*100))`、`auto_compact_token_limit = floor(input*0.9)` |
| `input_modalities` | `architecture.input_modalities` | `modalities.input` | `input_modalities`；并派生 `supports_image_detail_original`（含 `image` 才 true）。两边都只保留并去重 `text` / `image` / `audio` |
| `reasoning_levels` | `reasoning.supported_efforts[]` | `reasoning_options[type=effort].values[]` | `supported_reasoning_levels`（只写字段源给出的档位；描述文案另取基底表） |
| `default_reasoning_level` | `reasoning.default_effort`（小写） | —（不提供） | `default_reasoning_level`，仅当它落在档位列表里，否则 `null` |
| `description` | `description` | `description` | `description`（空则用占位文案「由模型提供者目录生成」） |
| `support_verbosity` | `supported_parameters` 含 `verbosity` | —（不提供） | `support_verbosity`；源明确给值时同时把 `default_verbosity` 写 `null` |
| `supports_search_tool` | `supported_parameters` 含 `web_search_options` 或 `web_search` | —（不提供） | `supports_search_tool` |
| `supports_tool_calls` | `supported_parameters` 含 `tools` | `tool_call`（bool） | **不写入目录**；可靠匹配最终采用的 `false` → `incompatible`，模糊值只提示继承对象限制 |
| `status` | `status`（去空白、小写） | `status`（去空白、小写） | 写入条目 `status`；`beta` → 警告；可靠匹配的 `deprecated` → `incompatible`，模糊值不直接禁用目标模型 |
| `supports_reasoning` | `reasoning` 存在且（`mandatory` / `supported_efforts` 非空 / `default_effort` 非空） | `reasoning === true` 或 `reasoning_options` 非空 | **目前无消费者**（只有单测断言），接上用途或删除前不必关注 |

> OpenRouter 的 `supported_parameters` 是「集合存在即为显式声明」：`verbosity` / `web_search*` / `tools`
> 不在合法字符串集合里即记 `false`（明确不支持，而不是未知）；数组类型或元素非法则视为未知。
> models.dev 用 `tool_call` 提供工具能力；verbosity 与搜索参数能力由 OpenRouter 提供。

### 只用于候选匹配、不进 ModelFacts 的字段

| 用途 | OpenRouter | models.dev |
| --- | --- | --- |
| 候选主键（输出 slug 仍用提供方返回的原始 ID） | `id` | `providers[*].models` 的 map key（条目缺 `id` 时用 key） |
| 权威别名（匹配级别 alias） | `canonical_slug`、`alias_target.slug` | 无；`family` 仅为模糊线索 |
| 发布时间（相似度相同时取新） | `created` | `release_date`，缺失回退 `last_updated` |
| 变体/实验标记（排序时靠后） | `status` | `status` |
| 数据形态 | `data[]` 或 `models[]`，每条必须有非空 `id` | `{ models, providers }`：保留同 ID 全部记录，逐字段处理权威性与冲突 |

### 合并与派生

- 来源各自独立查找，生成器通过 `FieldQuality` 的有序字段比较可信度，不再把来源分与匹配分相加。
  `FieldProvenance` 记录来源、继承 ID、匹配级别、资料所属 provider、最终值与处理原因。
- 明确 `false` 与空数组是有效值，缺失与解析失败才是未知。档位采用一个来源的完整列表，
  不拼接多个来源的集合；同来源多份记录仅顺序不同不算冲突。
- 档位过滤：只丢弃空值、`default`、`null` 哨兵，保留自定义档位；默认档位不在支持列表时自动清空。
- 输入上限大于上下文时，若输入上限更可信则用其重新派生上下文基准；否则放弃冲突输入限制。
  仅有输入上限、上下文未知时，以输入上限自动派生保守基准，不套用可能更大的模板窗口。
  随后校验正数上下文、最大上下文、有效比例与压缩阈值。必需字段类型非法或无法安全修复时，
  候选标 `invalid` 并隔离，不影响其他模型。

### 其余写入字段

| 字段 | 来源 |
| --- | --- |
| `default_reasoning_summary` | 推理条目（有档位或声明支持推理）写 `auto`、其余写 `none`；官方复用条目同样按此覆盖，否则 GPT 系只回加密推理、「思考过程」卡片为空 |
| `supports_reasoning_summary_parameter` | **一律不写**：官方条目没有该键，显式写 `false` 会让 codex 完全不请求推理摘要；复用条目里若残留 `false` 会被删除 |
| `slug` / `display_name` / `priority` | `slug` 用提供方返回的 ID；模板条目 `display_name` 按 slug 格式化；`priority` 按选中条目的候选原顺序从 1 重排，不按点击顺序 |
| 可用性 | 可靠匹配的 `tool_call=false` 或 `status=deprecated` → `incompatible`；条目校验失败 → `invalid`；这些条目不会写入目录 |

命令返回的每个候选包含 `status`（`ready` / `incompatible` / `unmatched` / `invalid`）、`selectable`、
`warnings` 与 `sources[]`。来源证据包括来源名、继承目标 `matched_id`、匹配类型与分数，
弹窗里**每个来源各展示一枚徽章**（未继承时只显示来源名，继承时显示「来源 · 继承 `matched_id`」）；
统计字段为 `ready / incompatible / unmatched / invalid`。即使全部不可生成，也会返回候选列表和
空目录 `{ "models": [] }`，弹窗展示具体原因并禁用确认，编辑框原内容不变。

选择弹窗、搜索、勾选、三态全选及确认步骤全部保留。字段级来源依据（`field_provenance`）只在后端
用于逐字段合并与校验，**不发给前端**，弹窗不提供参数来源展开、人工参数输入、来源选择或冲突确认。
用户点击确定后只过滤已选模型并重排优先级，
直接回填已自动计算的完整参数，不重新请求资料、不自动保存。取消、请求失败和迟到的过期响应
不会修改目录；保存仍走现有按钮。

### 官方条目的复用（本机 codex 导出 + 第三方条目）

官方条目池由两部分合并，**同 slug 时运行期导出条目优先**：

- **本机 codex 自带目录**：启动后后台跑一次 `codex debug models --bundled`，把该二进制自带的
  目录（GPT/codex 基线条目）原子写入 `%APPDATA%\com.codexui.app\cache\codex-models.json`；
  失败保留上次缓存并记一条 warn 日志。因此 GPT 条目始终与用户安装的 codex 版本匹配，
  老版本 codex 不会吃到新版本专用字段。`--bundled` 不读 `CODEX_HOME/config.toml`、不联网；
  解析只看 stdout（该命令会往 stderr 打 PATH 别名之类的 WARNING）。
- **`resources/official-models.json`**：只放手写的第三方官方条目（目前 `deepseek-flash`、
  `deepseek-v4-pro`），由 `scripts/update-model-catalog-sources.mjs --official` 校验并规范化
  （不联网；要求每条有 `slug` 与 `model_messages.instructions_template`，按 slug 去重）。

导出失败且无缓存时条目池只剩第三方条目：GPT 模型退回多源合并 + 模板渲染，
`codex-auto-review` 不再复用（不影响生成，只是少了官方口径的资料）。

先全池查找原始 slug 精确匹配，再查找唯一规范化匹配；规范化有歧义时自动转字段源。
命中时整条复用该条目：`context_window`、
`max_context_window`（如 `gpt-5.6-*` 的 872000）、`tool_mode=code_mode_only`、
`use_responses_lite`、`web_search_tool_type=text_and_image`、`include_*`、`multi_agent_version`、
`truncation_policy` 全部保留；只覆盖：

1. `slug` ← 提供方返回的原始 ID；
2. `priority` ← 勾选顺序；
3. 提供方主机不是 OpenAI 官方（`*.openai.com` / `*.chatgpt.com`）时，`prefer_websockets` 置 `false`。

**提示词按来源分流**：命中本源的条目（含唯一规范化匹配）保留自带提示词（`base_instructions`
与 `model_messages.instructions_template` 都不改）；**未命中**的模型走多源合并 + 模板渲染，
两个提示词字段都写成模板资源里的精简提示词（见下节）。

> **为什么要精简非本源模型的提示词**：模板基底取自官方条目，其提示词约 18K 字符；像
> `big-pickle` 这类只存在于提供方 `/models`、又不在导出池内的第三方模型，会带着这段
> 固定开销随会话增长把上游上下文顶满——实测表现为模型突然不再调用工具、回合静默结束，
> 随后上游开始回 500/400。精简提示词约 380 字符，保留「持续工作到完成、失败换方式重试、
> 先读后改、最小改动」等关键约束。

只做精确匹配、不做模糊匹配，避免把 `gpt-5.6` 之类乱映射到 `sol/terra/luna`。
因此想让某模型固定按你写的参数走，把它的条目放进这个文件即可（不再经历多源合并）。
`codex-auto-review` 不在任何提供方的 `/models` 里，使用自动评审时需手动加入目录。

## 模板：运行时基底 + 覆盖清单

**渲染模板 = 本机 codex 导出基底 + `resources/model_catalog_template.json` 覆盖**：

- 基底 = 启动时 `codex debug models --bundled` 导出的**第一条**（缓存 `cache/codex-models.json`
  ＋进程内快照，见下文「数据源与资源刷新」），因此生成条目的**键集与用户实际安装的
  codex 版本一致**；基底里的 `base_instructions` 会被丢弃，再由覆盖清单写入精简版。
- 覆盖清单 = `src-tauri/resources/model_catalog_template.json`（44 键）：有同名键就用清单里的值，
  其余键（含新版本新增的键）跟随基底。改固定行为 = 只改这个文件，`template.rs` 的护栏测试会拦住误改。
  其中 `base_instructions` 与 `model_messages.instructions_template` 写同一段精简提示词，
  服务所有未命中完整条目源的模型；`model_messages` 只覆盖 `instructions_template`，
  `approvals` / `collaboration_modes` / `instructions_variables` / `multi_agent` / `permissions`
  等子键在合并时保留基底的（整对象替换会丢）。
- 拿不到导出时（老版本 codex 没有 `debug models`、或导出失败且无缓存）基底改用
  `resources/model_catalog_fallback.json`（11 个必需键骨架 + 一份提示词），其余仍由覆盖清单决定。
- 写出前还会对**每一条**输出条目（模板渲染条目与官方复用条目，含手写第三方条目）做
  「只补不覆盖」的补键，保证不出现缺字段；`base_instructions` 是唯一例外——它只由模板渲染
  路径产出，**不**向复用条目补齐，否则命中完整条目源的模型会被塞进精简基底、悄悄换掉自带提示词。

> **为什么必须对齐**：codex 解析 `model_catalog_json` 时缺必需字段是**硬失败**——
> `{"models":[{"slug":"x"}]}` 会直接报 `missing field display_name`、缺 `visibility` 同样报错，
> 整份目录都加载不了。0.149.0 逐个删字段实测：必需键共 11 个
> （`slug`、`display_name`、`priority`、`model_messages`、`supported_reasoning_levels`、
> `support_verbosity`、`shell_type`、`truncation_policy`、`visibility`、`supported_in_api`、
> `experimental_supported_tools`），其余 31 个删掉都能解析（多出来的未知键会被 serde 忽略）。
> 渲染器同样**不再删键**：`default_reasoning_level`、`default_verbosity` 无值写 `null`
> （实测 null 可解析），避免这些键在某个版本变成必需时整份目录失效。

关键固定值：`prefer_websockets=false`、`web_search_tool_type="text"`、`use_responses_lite=false`、
`tool_mode=null`（协议层最小化，第三方 provider 不走 OpenAI 专用传输）；
`truncation_policy={tokens,10000}`、`multi_agent_version="v2"`、`comp_hash="3000"`、
`minimal_client_version="0.144.0"`、`reasoning_summary_format="experimental"`、
`default_reasoning_summary="auto"`（渲染器按推理能力改写为 `auto` / `none`）、`supports_reasoning_summaries=true`、
`include_skills_usage_instructions=false`、`include_plugin_usage_instructions=true`、
`include_apps_usage_instructions=true`、`effective_context_window_percent=95`、
`supports_parallel_tool_calls=true`、`shell_type="shell_command"`、`apply_patch_tool_type="freeform"`、
`visibility="list"`、`supported_in_api=true`、`available_in_plans=[]`、`service_tiers=[]`、
`additional_speed_tiers=[]`、`default_service_tier=null`。

提示词由覆盖清单持有（`base_instructions` 与 `model_messages.instructions_template` 同写精简版，
服务未命中完整条目源的模型；命中本源的条目保留自带提示词）；`supported_reasoning_levels` 在覆盖清单里
是**最小化空数组 `[]`**——只保证 Key 存在（它是必需字段、也是补键时的安全兜底值），
不携带任何档位；渲染期的档位**描述**改从**基底条目**的表里查（本机 codex / 兜底资源），
查不到才退化为 `"{effort} reasoning effort"`。条目里的档位列表始终来自字段源，
无档位信息就是 `[]`，不会继承模板或基底的档位。

> 注：`supports_parallel_tool_calls`、`reasoning_summary_format`、`minimal_client_version`、
> `supports_reasoning_summaries`、`available_in_plans` 在 0.149.0 的 `ModelInfo` 里并不存在，
> 会被 serde 忽略；保留它们是为了与官方条目形态一致并向前兼容。

## 数据源与资源刷新

| 资源 | 用途 | 更新方式 |
| --- | --- | --- |
| `resources/official-models.json` | 第三方官方条目池（完整条目源）：只放手写的第三方厂商条目（deepseek 等）；GPT/codex 基线条目改为启动时从本机 codex 导出 | `--official`（校验 + 规范化，不联网） |
| `resources/models-dev.json` | models.dev 内置快照（`catalog.json` 精简版）：`providers` **全量** 213 个 provider / 7669 个模型 + `models` 382 条模型级条目，约 3 MB，保留 `tool_call` / `status` / `reasoning_options`；省略与 map key 重复的 `id` 与不再使用的 `api` / `name` | 同上 |
| `resources/openrouter-models.json` | OpenRouter 全量响应回退 | 同上 |
| `resources/model_catalog_template.json` | 渲染模板的**覆盖清单**（44 键）：固定值、占位值与精简提示词（`base_instructions` + `model_messages.instructions_template`） | `--template` |
| `resources/model_catalog_fallback.json` | 无本机 codex 导出时的基底骨架（11 个必需键 + 提示词） | `--template` |

运行时缓存位于应用数据目录 `%APPDATA%\com.codexui.app\cache\`：`openrouter-models.json`
与 `models-dev.json` **均为 24 小时 TTL**——启动时检查各自文件的 mtime，未过期直接复用、
不重复下载；缓存缺失、过期或**内容损坏（无法解析）**时才重新抓取，抓取失败保留旧缓存；
读取时若缓存不可用则回退内置资源。缓存里保存的是抓取到的原始完整响应
（models.dev `catalog.json` 约 4.9 MB，不含精简；旧版 `api.json` 形态的缓存因缺少
`providers` 键会被判为过期并重新抓取）。

同目录另有 `codex-models.json`：**每次启动都后台重新导出**（`codex debug models --bundled`），
校验通过才原子替换；导出失败/超时/形状非法时保留上一次的文件，缓存损坏等同于没有导出。
同路径导出串行发布，写盘成功后更新内存；首次读盘不会覆盖后台刚发布的新快照。
一次生成只捕获一个不可变快照，官方条目池和模板均使用它。导出超时后终止并回收子进程，
stdout/stderr 并行排空，不留下后台读取任务。

```powershell
node scripts/update-model-catalog-sources.mjs                # 全部
node scripts/update-model-catalog-sources.mjs --official     # 校验并规范化第三方官方条目（不联网）
node scripts/update-model-catalog-sources.mjs --template     # 生成基底模板（从 codex 基线派生）
node scripts/update-model-catalog-sources.mjs --models-dev   # models.dev 精简快照
node scripts/update-model-catalog-sources.mjs --openrouter   # OpenRouter 全量响应
```
