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
   ├─ 1. 完整条目源（official）：本机 codex 导出条目 + 内置第三方条目，命中即整条复用，不套模板
   │
   └─ 2. 字段源：models.dev + OpenRouter
          │  每个源返回匹配类型、分数与 ModelFacts
          │  merge 按来源权威性和匹配质量逐字段选择，低质量源只补缺失值
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

- **权威别名**：OpenRouter 用 `canonical_slug` / `alias_target.slug`；models.dev 用 `family`。
- **匹配级别**：规范模型 ID 精确 → 权威别名精确 → 规范化精确 → 同系列模糊匹配 → 无匹配。
  一般模糊阈值为 0.72；版本系列已确认一致时可降到 0.60，容纳 `coder` / `flash` / `pro`
  等规格后缀造成的额外距离。
  规范化会剥掉 provider 前缀、`:free` / `:batch` / `-free` / `-latest` / `-preview` /
  `-beta` / `-alpha` 后缀与结尾日期。
- **模糊排序**（依次比较，全部确定性）：同厂商 → 同版本系列 → 保留查询中的规格 token
  （`coder` / `chat` / `reasoner` / `flash` / `pro` / `mini` / `sonnet` 等）→
  正式模型优先于实验/beta/deprecated → 版本距离近者（同系列标记后的首个整数，
  如查询 `deepseek-v5` 时 `deepseek-v4-*` 优于中转商的历史 `deepseek-v3*`）→
  多余 token 少者 → 相似度 → 发布时间新者 → 模型 ID。
- **效果**：`gpt-5.7` 继承最新正式 GPT-5.x，`qwen4-coder` 优先继承 Qwen Coder，
  `deepseek-v5` 只在 DeepSeek V 系列内选择；模糊命中仍是正常可生成模型，只在弹窗显示
  “资料继承自……”的溯源。
- **输出 slug 始终是提供方返回的原始 ID**，只有能力字段来自命中的规范模型。
- **只按模型 ID 匹配**（models.dev）：不按 `base_url` 定位 provider 分组，也不看提供方，
  同一个模型 ID 在任何提供方下的结果一致。索引由 `catalog.json` 的 `providers` 全部分组
  展平得到（同 ID 取分组 id 字典序第一份），`models` 里的模型级条目只补 `providers`
  未覆盖的 ID（如 `swiss-ai/apertus-8b`）——它没有 `reasoning_options` / `status`，
  因此不反压分组资料，推理档位与 beta/deprecated 判定不退化。
- **字段覆盖**：models.dev 基础分高于 OpenRouter，两者再按匹配级别（精确/别名/规范化/模糊）
  加分比较；低质量命中只能补空字段，不能覆盖高质量值（各源取值不同不再作为警告展示）。

## 字段来源

生成器写入的字段只有两类：**数据驱动**（来自字段源）、**固定值**（来自模板）。

| 字段 | 来源 |
| --- | --- |
| `context_window` / `max_context_window` | models.dev `limit.context` 或 OpenRouter `context_length`；两者都表示总上下文 |
| `effective_context_window_percent` / `auto_compact_token_limit` | 若 models.dev `limit.input < limit.context`：分别为 `min(95, floor(input/context*100))` 与 `floor(input*0.9)` |
| `input_modalities` | models.dev `modalities.input` → OpenRouter `architecture.input_modalities`（过滤为 text/image/audio） |
| `supported_reasoning_levels` | models.dev `reasoning_options[type=effort].values` / OpenRouter `reasoning.supported_efforts`；保留合法自定义值，只过滤空值、`default`、`null` 哨兵 |
| `default_reasoning_level` | 仅数据源明确给出且该值存在于档位列表时写入，不自行猜测 |
| `description` | models.dev → OpenRouter → 占位文案 |
| `supports_reasoning_summary_parameter` | 模板生成条目固定为 false；官方完整条目保持原值 |
| `support_verbosity` / `default_verbosity` | 默认 false 且不写默认 verbosity；仅数据源明确声明 `verbosity` 时开启 |
| `supports_search_tool` | 默认 false；仅数据源明确声明 `web_search_options` / `web_search` 时开启 |
| `status` | models.dev 的有效状态写入生成条目；`beta` 正常生成并显示警告 |
| 可用性 | models.dev `tool_call=false` 或 `status=deprecated` → `incompatible`；不兼容条目不会写入目录 |
| `supports_image_detail_original` | 派生：`input_modalities` 含 `image` 则为 true |
| `slug` / `display_name` / `priority` | `slug` 用提供方返回的 ID；`display_name` 按 slug 格式化；`priority` 按勾选顺序从 1 重排 |

命令返回的每个候选包含 `status`（`ready` / `incompatible` / `unmatched`）、`selectable`、
`warnings` 与 `sources[]`。来源证据包括来源名、继承目标 `matched_id`、匹配类型与分数，
弹窗里**每个来源各展示一枚徽章**（未继承时只显示来源名，继承时显示「来源 · 继承 `matched_id`」）；
统计字段为 `ready / incompatible / unmatched`。即使全部未匹配或不兼容，也会返回候选列表和
空目录 `{ "models": [] }`，由弹窗展示具体原因。

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

命中池中 slug（精确或规范化相同）时整条复用该条目：专属提示词、`context_window`、
`max_context_window`（如 `gpt-5.6-*` 的 872000）、`tool_mode=code_mode_only`、
`use_responses_lite`、`web_search_tool_type=text_and_image`、`include_*`、`multi_agent_version`、
`truncation_policy` 全部保留；只覆盖：

1. `slug` ← 提供方返回的原始 ID；
2. `priority` ← 勾选顺序；
3. 提供方主机不是 OpenAI 官方（`*.openai.com` / `*.chatgpt.com`）时，`prefer_websockets` 置 `false`。

只做精确匹配、不做模糊匹配，避免把 `gpt-5.6` 之类乱映射到 `sol/terra/luna`。
因此想让某模型固定按你写的参数走，把它的条目放进这个文件即可（不再经历多源合并）。
`codex-auto-review` 不在任何提供方的 `/models` 里，使用自动评审时需手动加入目录。

## 模板（固定值的唯一事实来源）

模板 `src-tauri/resources/model_catalog_template.json`（42 键）由脚本从 codex 基线
`models.json` 的 `gpt-5.6-sol` 派生（不再依赖内置条目池），并做传输层最小化与占位。
改固定行为 = 只改这个文件（代码里没有固定值常量），
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
| `resources/official-models.json` | 第三方官方条目池（完整条目源）：只放手写的第三方厂商条目（deepseek 等）；GPT/codex 基线条目改为启动时从本机 codex 导出 | `--official`（校验 + 规范化，不联网） |
| `resources/models-dev.json` | models.dev 内置快照（`catalog.json` 精简版）：`providers` **全量** 213 个 provider / 7669 个模型 + `models` 382 条模型级条目，约 3 MB，保留 `tool_call` / `status` / `reasoning_options`；省略与 map key 重复的 `id` 与不再使用的 `api` / `name` | 同上 |
| `resources/openrouter-models.json` | OpenRouter 全量响应回退 | 同上 |
| `resources/model_catalog_template.json` | 渲染基底（由 codex 基线 `models.json` 的 `gpt-5.6-sol` 派生） | `--template` |

运行时缓存位于应用数据目录 `%APPDATA%\com.codexui.app\cache\`：`openrouter-models.json`
与 `models-dev.json` **均为 24 小时 TTL**——启动时检查各自文件的 mtime，未过期直接复用、
不重复下载；缓存缺失、过期或**内容损坏（无法解析）**时才重新抓取，抓取失败保留旧缓存；
读取时若缓存不可用则回退内置资源。缓存里保存的是抓取到的原始完整响应
（models.dev `catalog.json` 约 4.9 MB，不含精简；旧版 `api.json` 形态的缓存因缺少
`providers` 键会被判为过期并重新抓取）。

同目录另有 `codex-models.json`：**每次启动都后台重新导出**（`codex debug models --bundled`），
校验通过才原子替换；导出失败/超时/形状非法时保留上一次的文件，缓存损坏等同于没有导出。

```powershell
node scripts/update-model-catalog-sources.mjs                # 全部
node scripts/update-model-catalog-sources.mjs --official     # 校验并规范化第三方官方条目（不联网）
node scripts/update-model-catalog-sources.mjs --template     # 生成基底模板（从 codex 基线派生）
node scripts/update-model-catalog-sources.mjs --models-dev   # models.dev 精简快照
node scripts/update-model-catalog-sources.mjs --openrouter   # OpenRouter 全量响应
```
