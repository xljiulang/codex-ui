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
- **条目字段不完整会退化**：字段缺失会走 codex 的 fallback 元数据（上下文 272k、
  `visibility=none`），所以生成的条目始终输出完整键集。

## 生成管线

```
提供方 /models（模型 ID 列表）
   │
   ├─ 1. 完整条目源（official）：命中即整条复用，不套模板
   │
   └─ 2. 字段源：openrouter（基础）→ models_dev（覆盖）
          │  每个源只做「提取 → 记录」（ModelFacts），覆盖由 merge 逐字段完成
          ▼
      3. 模板渲染（model_catalog_template.json，固定值唯一事实来源）
```

代码结构（`src-tauri/src/codex/model_catalog/`）：

| 文件 | 职责 |
| --- | --- |
| `mod.rs` | 管线编排、命令入口、`refresh_source_caches`、`ModelCatalogGenerateResult` |
| `facts.rs` | `ModelFacts`（全字段可选）+ `merge_from` 逐字段覆盖 + `provenance` 溯源 |
| `matching.rs` | 别名键匹配、匹配级别与排序键、`CandidateStore`、规范化/相似度 |
| `template.rs` | 模板加载、`DATA_DRIVEN_KEYS`、四条派生规则 |
| `sources/official.rs` | 完整条目源：官方条目池（精确匹配） |
| `sources/models_dev.rs` | 字段源（provider 主机推断 + 全局索引） |
| `sources/openrouter.rs` | 字段源（缓存 + 内置回退） |

### 新增一个数据源（三步）

1. 实现 trait：`FactSource`（给部分字段）或 `FullEntrySource`（给完整条目）；
2. 在 `sources/mod.rs` 的 `fact_sources()` / `full_entry_sources()` 里按优先级追加一行；
3. 加映射单测 + 在 `scripts/update-model-catalog-sources.mjs` 里补抓取/精简逻辑（若需要内置快照）。

合并、匹配、渲染逻辑都不需要改动。

## 匹配与排序规则

- **别名键**：OpenRouter 用 `id` / `canonical_slug`；models.dev 用 `id` / `family`。
- **匹配级别**：别名键精确 → 规范化精确 → 相似度 ≥ 0.72。
  规范化会剥掉 provider 前缀、`:free` / `:batch` / `-free` / `-latest` / `-preview` /
  `-beta` / `-alpha` 后缀与结尾日期。
- **同级别排序**（依次比较，全部确定性）：
  1. `is_variant` 为 false 优先 —— 原始键含 `exp`/`experimental`/`preview`/`beta`/`alpha`/
     `rc`/`nightly`/`dev`/`free`/`batch`/`latest` 任一 token 即视为变体或别名（`vision` 不算）；
  2. 多余 token 少者优先（候选键比查询多出的 token 数）；
  3. 发布时间新者优先（models.dev `release_date` → `last_updated`，OpenRouter `created`）；
  4. 候选 id 字典序兜底。
- **效果**：无版本 ID（`deepseek-flash`）先按 `family` 命中同族候选，实验变体落败后取
  `deepseek-v4-flash`；同族出现 `v4`/`v5` 两个同构版本时取发布时间最新的；带版本查询
  （`deepseek-v4-flash-0731`）因多余 token 最少仍命中自身。
- **输出 slug 始终是提供方返回的原始 ID**，只有能力字段来自命中的规范模型。
- **provider 分组定位**（仅 models.dev）：先按 `base_url` 与 provider `api` 的
  同源 + 路径前缀匹配（区分同主机不同产品线，如 `https://opencode.ai/zen/v1` → `opencode`、
  `https://opencode.ai/zen/go/v1` → `opencode-go`），无前缀命中时退化为同主机匹配；
  命中多个时取路径最长者、再按 provider id。该分组内匹配不中时，才回退全量索引。
- **全量索引同 ID 冲突**：优先 `models-dev-official-providers.json` 里的官方厂商
  （如 `deepseek`），其余按 provider id 字典序；保证中转商不会凭字母序覆盖官方参数。

## 字段来源

生成器写入的字段只有两类：**数据驱动**（来自字段源）、**固定值**（来自模板）。

| 字段 | 来源 |
| --- | --- |
| `context_window` / `max_context_window` | models.dev `limit.context` → OpenRouter `context_length` |
| `input_modalities` | models.dev `modalities.input` → OpenRouter `architecture.input_modalities`（过滤为 text/image/audio） |
| `supported_reasoning_levels` | models.dev `reasoning_options[type=effort].values` → OpenRouter `reasoning.supported_efforts`（过滤到 codex 已知档位） |
| `default_reasoning_level` | OpenRouter `reasoning.default_effort`（须在档位内），否则按 `medium→high→low→xhigh→max→minimal→none` 取首个存在的；档位为空则删键 |
| `description` | models.dev → OpenRouter → 占位文案 |
| `supports_reasoning_summary_parameter` | 任一源声明推理能力即为 true，否则 false |
| `support_verbosity` / `default_verbosity` | 源明确声明时用源（明确 false 会删 `default_verbosity`），源无信息时保留模板值 |
| `supports_search_tool` | 同上（OpenRouter 看 `supported_parameters` 的 `web_search_options`/`web_search`） |
| `supports_image_detail_original` | 派生：`input_modalities` 含 `image` 则为 true |
| `slug` / `display_name` / `priority` | `slug` 用提供方返回的 ID；`display_name` 按 slug 格式化；`priority` 按勾选顺序从 1 重排 |

### 官方条目的复用（codex GPT / deepseek 等厂商同池）

`resources/official-models.json` 是**多厂商共用的官方条目池**：codex 基线条目由
`scripts/update-model-catalog-sources.mjs --official` 刷新，其它厂商（如 deepseek）提供的
条目可直接手工追加——脚本刷新时按 slug 保留这些手写条目。

命中池中 slug（精确或规范化相同）时整条复用该条目：专属提示词、`context_window`、
`max_context_window`（如 `gpt-5.6-*` 的 872000）、`tool_mode=code_mode_only`、
`use_responses_lite`、`web_search_tool_type=text_and_image`、`include_*`、`multi_agent_version`、
`truncation_policy`、`comp_hash`、`minimal_client_version` 全部保留；只覆盖：

1. `slug` ← 提供方返回的原始 ID；
2. `priority` ← 勾选顺序；
3. 提供方主机不是 OpenAI 官方（`*.openai.com` / `*.chatgpt.com`）时，`prefer_websockets` 置 `false`。

只做精确匹配、不做模糊匹配，避免把 `gpt-5.6` 之类乱映射到 `sol/terra/luna`。
因此想让某模型固定按你写的参数走，把它的条目放进这个文件即可（不再经历多源合并）。
`codex-auto-review` 不在任何提供方的 `/models` 里，使用自动评审时需手动加入目录。

## 模板（固定值的唯一事实来源）

模板 `src-tauri/resources/model_catalog_template.json`（43 键）由脚本从官方条目池的
`gpt-5.6-sol` 派生，并做传输层最小化与占位。改固定行为 = 只改这个文件（代码里没有固定值常量），
`src-tauri/src/codex/model_catalog/template.rs` 的护栏测试会拦住误改。

关键固定值：`prefer_websockets=false`、`web_search_tool_type="text"`、`use_responses_lite=false`、
`tool_mode=null`（协议层最小化，第三方 provider 不走 OpenAI 专用传输）；
`truncation_policy={tokens,10000}`、`multi_agent_version="v2"`、`comp_hash="3000"`、
`minimal_client_version="0.144.0"`、`reasoning_summary_format="experimental"`、
`default_reasoning_summary="none"`、`supports_reasoning_summaries=true`、
`include_skills_usage_instructions=false`、`include_plugin_usage_instructions=true`、
`include_apps_usage_instructions=true`、`effective_context_window_percent=95`、
`supports_parallel_tool_calls=true`、`shell_type="shell_command"`、`apply_patch_tool_type="freeform"`、
`visibility="list"`、`supported_in_api=true`、`available_in_plans=[]`、`service_tiers=[]`、
`additional_speed_tiers=[]`、`default_service_tier=null`。

提示词只保留 `model_messages.instructions_template`（不再重复写 `base_instructions`，
官方内置条目也没有该键）；`supported_reasoning_levels` 在模板里保留一份档位描述表，
渲染时必被覆盖，仅用于查档位描述。

> 注：`supports_parallel_tool_calls`、`reasoning_summary_format`、`minimal_client_version`、
> `supports_reasoning_summaries`、`available_in_plans` 在 0.149.0 的 `ModelInfo` 里并不存在，
> 会被 serde 忽略；保留它们是为了与官方条目形态一致并向前兼容。

## 数据源与资源刷新

| 资源 | 用途 | 更新方式 |
| --- | --- | --- |
| `resources/official-models.json` | 官方条目池（完整条目源）：codex 基线条目与 codex 版本绑定，可手工追加其它厂商条目 | `--official` |
| `resources/models-dev.json` | models.dev 内置快照：**全量** provider 分组（213 个 provider / 7616 个模型，约 2.8 MB，含中转/聚合商，首次离线也能匹配 zen 之类中转） | 同上 |
| `resources/models-dev-official-providers.json` | 官方厂商 id 清单（约 0.5 KB），只用于全局索引同 ID 冲突时的优先级 | 同上 |
| `resources/openrouter-models.json` | OpenRouter 全量响应回退 | 同上 |
| `resources/model_catalog_template.json` | 渲染基底（由官方条目池的 `gpt-5.6-sol` 派生） | `--template` |

运行时缓存位于应用数据目录 `%APPDATA%\com.codexui.app\cache\`：`openrouter-models.json`
与 `models-dev.json` **均为 24 小时 TTL**——启动时检查各自文件的 mtime，未过期直接复用、
不重复下载；缓存缺失、过期或**内容损坏（无法解析）**时才重新抓取，抓取失败保留旧缓存；
读取时若缓存不可用则回退上面两份内置资源。缓存里保存的是抓取到的原始完整响应
（models.dev 约 4.5 MB，不含精简）。

```powershell
node scripts/update-model-catalog-sources.mjs                # 全部
node scripts/update-model-catalog-sources.mjs --official     # 官方条目池（保留手写条目）
node scripts/update-model-catalog-sources.mjs --template     # 生成基底模板
node scripts/update-model-catalog-sources.mjs --models-dev   # models.dev 精简快照
node scripts/update-model-catalog-sources.mjs --openrouter   # OpenRouter 全量响应
```
