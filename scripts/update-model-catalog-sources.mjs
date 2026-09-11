#!/usr/bin/env node
// 更新模型目录生成用的内置资源（src-tauri/resources/）。
//
//   node scripts/update-model-catalog-sources.mjs                # 全部
//   node scripts/update-model-catalog-sources.mjs --official     # 官方条目（合并刷新 codex 基线，保留手写条目）
//   node scripts/update-model-catalog-sources.mjs --template     # 生成基底模板（依赖官方条目）
//   node scripts/update-model-catalog-sources.mjs --models-dev   # models.dev 全量精简快照 + 官方厂商清单
//   node scripts/update-model-catalog-sources.mjs --openrouter   # OpenRouter 全量响应
//
// 各资源的地位不同：
// - official-models.json：多厂商官方条目池（命中即整条复用）；codex 基线条目由本脚本刷新，
//   其它厂商（如 deepseek）可直接手工追加，刷新时会保留；
// - models-dev.json：全量精简快照（含中转/聚合商，保证首次离线也能匹配）；
// - models-dev-official-providers.json：官方厂商清单，仅用于同 ID 冲突时的优先级；
// - model_catalog_template.json：字段源合并后的渲染基底，固定值全部固化在这里。

import { mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const RESOURCES = join(ROOT, "src-tauri", "resources");

/** codex 基线版本：与 src-tauri 里适配的 codex-cli 版本保持一致。 */
const CODEX_BASELINE_TAG = "rust-v0.149.0";
const CODEX_MODELS_URL = `https://raw.githubusercontent.com/openai/codex/${CODEX_BASELINE_TAG}/codex-rs/models-manager/models.json`;
const MODELS_DEV_URL = "https://models.dev/api.json";
const OPENROUTER_URL = "https://openrouter.ai/api/v1/models";

/** 模板基底条目：取内置目录里最新一代的可见模型。 */
const TEMPLATE_BASE_SLUG = "gpt-5.6-sol";

/**
 * 官方厂商清单（唯一来源）：只用于「同 ID 冲突时优先取官方厂商参数」，
 * 不参与快照过滤——快照是全量的，中转商/聚合商照收。
 */
const OFFICIAL_VENDORS = [
  "openai",
  "anthropic",
  "google",
  "google-vertex",
  "xai",
  "meta",
  "mistral",
  "cohere",
  "perplexity",
  "nvidia",
  "amazon-bedrock",
  "azure",
  "deepseek",
  "moonshotai",
  "moonshotai-cn",
  "zhipuai",
  "zai",
  "minimax",
  "minimax-cn",
  "alibaba",
  "alibaba-cn",
  "qwen",
  "xiaomi",
  "stepfun",
  "longcat",
  "baidu",
  "tencent-tokenhub",
  "inception",
  "thinkingmachines",
  "poolside",
  "sakana",
  "upstage",
  "ai21",
];

/**
 * 精简快照保留的模型字段：只留映射需要的能力参数。
 * 冗余的 `id` 与 map key 重复，故省略（Rust 侧缺失时用 map key 注入）。
 */
const MODEL_KEEP_KEYS = [
  "description",
  "family",
  "reasoning",
  "reasoning_options",
  "modalities",
  "limit",
  "tool_call",
  "status",
  "release_date",
  "last_updated",
];
const DESCRIPTION_MAX = 300;

const args = new Set(process.argv.slice(2));
const runAll = args.size === 0;
const shouldRun = (flag) => runAll || args.has(flag);

async function fetchJson(url) {
  const response = await fetch(url, { headers: { Accept: "application/json" } });
  if (!response.ok) {
    throw new Error(`请求失败 ${response.status} ${response.statusText}: ${url}`);
  }
  return response.text();
}

async function writeResource(name, text) {
  await mkdir(RESOURCES, { recursive: true });
  await writeFile(join(RESOURCES, name), text, "utf8");
  console.log(`写入 ${name}（${text.length} 字节）`);
}

async function readResource(name) {
  return readFile(join(RESOURCES, name), "utf8");
}

function truncate(value, max) {
  if (typeof value !== "string" || value.length <= max) return value ?? null;
  return value.slice(0, max);
}

/**
 * 刷新官方条目池里的 codex 基线条目（条目形态即 model_catalog_json 的权威形状）。
 *
 * 合并写回：本次抓取到的 slug 之外的手写条目（其它厂商提供的数据）原样保留，
 * 排在抓取条目之后，并打印保留清单——这样往 official-models.json 里追加条目是安全的。
 */
async function updateOfficial() {
  const text = await fetchJson(CODEX_MODELS_URL);
  const fetched = JSON.parse(text);
  if (!Array.isArray(fetched.models) || fetched.models.length === 0) {
    throw new Error("codex 基线目录缺少 models 数组");
  }
  for (const model of fetched.models) {
    if (typeof model.slug !== "string" || !model.slug) {
      throw new Error("codex 基线目录存在缺少 slug 的条目");
    }
  }

  const fetchedSlugs = new Set(fetched.models.map((model) => model.slug));
  const kept = await readExistingEntries(fetchedSlugs);
  if (kept.length > 0) {
    console.log(
      `保留手写条目 ${kept.length} 条：${kept.map((model) => model.slug).join(", ")}`,
    );
  }

  const models = [...fetched.models, ...kept];
  await writeResource("official-models.json", `${JSON.stringify({ ...fetched, models }, null, 2)}\n`);
}

/** 读现有官方条目文件，返回本次抓取范围之外的条目；文件缺失/损坏时返回空数组。 */
async function readExistingEntries(fetchedSlugs) {
  let existing;
  try {
    existing = JSON.parse(await readFile(join(RESOURCES, "official-models.json"), "utf8"));
  } catch {
    return [];
  }
  if (!Array.isArray(existing?.models)) return [];
  const kept = [];
  const invalid = [];
  for (const model of existing.models) {
    const slug = typeof model?.slug === "string" ? model.slug : "";
    if (!slug) {
      invalid.push(model);
      continue;
    }
    if (!fetchedSlugs.has(slug)) kept.push(model);
  }
  if (invalid.length > 0) {
    console.warn(
      `警告：official-models.json 有 ${invalid.length} 条缺少 slug 的条目，已跳过（是否把整个 models.json 当成单条粘进来了？）`,
    );
  }
  return kept;
}

/**
 * 由官方条目池里的一个条目推导基底模板：
 * 固定值全部固化在模板里，代码不持有任何固定取值；
 * 数据驱动字段只留占位值，生成时由字段源覆盖；其中 supported_reasoning_levels
 * 保留基底条目的档位与描述，作为渲染期的“档位描述表”（写入前必被覆盖，不会泄漏）。
 */
async function updateTemplate() {
  const text = await readResource("official-models.json");
  const official = JSON.parse(text);
  const base =
    official.models.find((model) => model.slug === TEMPLATE_BASE_SLUG) ?? official.models[0];
  const template = structuredClone(base);

  // 提示词只保留 model_messages.instructions_template（官方条目从不写 base_instructions）
  delete template.base_instructions;

  // 数据驱动占位：生成时一律覆盖
  template.slug = "placeholder-model";
  template.display_name = "Placeholder-Model";
  template.description = null;
  template.priority = 99;
  template.input_modalities = ["text"];
  template.default_reasoning_level = null;
  template.context_window = 272000;
  template.max_context_window = 272000;
  template.support_verbosity = false;
  delete template.default_verbosity;
  template.supports_search_tool = false;
  template.supports_image_detail_original = false;

  // 传输层最小化：第三方 provider 不走 OpenAI 专用传输与工具形态
  template.prefer_websockets = false;
  template.web_search_tool_type = "text";
  template.use_responses_lite = false;
  template.tool_mode = null;

  // OpenAI 套餐/速度档位信息对第三方 provider 无意义，清空
  template.available_in_plans = [];
  template.service_tiers = [];
  template.additional_speed_tiers = [];
  template.default_service_tier = null;

  // 0.149.0 ModelInfo 里的显式默认值（官方条目不写，等于 95）
  template.effective_context_window_percent = 95;

  // 旧 codex 版本忽略、新版本会读取的前向兼容字段
  if (!("supports_parallel_tool_calls" in template)) {
    template.supports_parallel_tool_calls = true;
  }

  await writeResource(
    "model_catalog_template.json",
    `${JSON.stringify(template, null, 2)}\n`,
  );
}

/** models.dev 全量目录 → 精简字段版本（保留全部 provider，形态与全量一致） */
async function updateModelsDev() {
  const text = await fetchJson(MODELS_DEV_URL);
  const parsed = JSON.parse(text);
  const slim = {};
  let providers = 0;
  let models = 0;

  for (const [providerId, provider] of Object.entries(parsed)) {
    if (!provider || typeof provider !== "object" || !provider.models) continue;
    const slimModels = {};
    for (const [modelId, model] of Object.entries(provider.models)) {
      if (!model || typeof model !== "object") continue;
      const keep = {};
      for (const key of MODEL_KEEP_KEYS) {
        if (key in model) keep[key] = key === "description" ? truncate(model[key], DESCRIPTION_MAX) : model[key];
      }
      slimModels[modelId] = keep;
      models += 1;
    }
    if (Object.keys(slimModels).length === 0) continue;
    slim[providerId] = {
      id: provider.id ?? providerId,
      api: provider.api ?? null,
      name: provider.name ?? providerId,
      models: slimModels,
    };
    providers += 1;
  }

  if (providers === 0) throw new Error("models.dev 目录没有可用的 provider 分组");
  console.log(`models.dev 全量精简：${providers} 个 provider、${models} 个模型`);
  await writeResource("models-dev.json", `${JSON.stringify(slim)}\n`);

  // 官方厂商清单：只保留确实存在于本次抓取结果里的 id，排序保证可复现
  const official = OFFICIAL_VENDORS.filter((id) => id in slim).sort();
  console.log(`官方厂商清单：${official.length} 个（${official.join(", ")}）`);
  await writeResource(
    "models-dev-official-providers.json",
    `${JSON.stringify(official, null, 2)}\n`,
  );
}

/** OpenRouter 全量响应（含 data[].id / canonical_slug / created 等）。 */
async function updateOpenRouter() {
  const text = await fetchJson(OPENROUTER_URL);
  const parsed = JSON.parse(text);
  if (!Array.isArray(parsed.data) || parsed.data.length === 0) {
    throw new Error("OpenRouter 响应缺少 data 数组");
  }
  await writeResource("openrouter-models.json", text.trim());
}

async function main() {
  if (shouldRun("--official")) await updateOfficial();
  if (shouldRun("--template")) await updateTemplate();
  if (shouldRun("--models-dev")) await updateModelsDev();
  if (shouldRun("--openrouter")) await updateOpenRouter();
}

main().catch((error) => {
  console.error(error instanceof Error ? error.message : error);
  process.exitCode = 1;
});
