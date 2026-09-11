#!/usr/bin/env node
// 更新模型目录生成用的内置资源（src-tauri/resources/）。
//
//   node scripts/update-model-catalog-sources.mjs                # 全部
//   node scripts/update-model-catalog-sources.mjs --official     # 校验并规范化手写的第三方官方条目（不联网）
//   node scripts/update-model-catalog-sources.mjs --template     # 生成基底模板（从 codex 基线条目派生）
//   node scripts/update-model-catalog-sources.mjs --models-dev   # models.dev 精简快照（provider 分组 + 模型清单）
//   node scripts/update-model-catalog-sources.mjs --openrouter   # OpenRouter 全量响应
//
// 各资源的地位不同：
// - official-models.json：第三方官方条目池（命中即整条复用，目前是 deepseek 等手写条目）；
//   GPT/codex 基线条目不再内置——应用启动时用 `codex debug models --bundled` 从用户安装的
//   codex 导出，运行时与本文件合并（同 slug 时运行期条目优先）；
// - models-dev.json：全量精简快照（含中转/聚合商，保证首次离线也能匹配）；
// - model_catalog_template.json：字段源合并后的渲染基底，固定值全部固化在这里。

import { mkdir, readFile, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const ROOT = join(dirname(fileURLToPath(import.meta.url)), "..");
const RESOURCES = join(ROOT, "src-tauri", "resources");

/** codex 基线版本：与 src-tauri 里适配的 codex-cli 版本保持一致。 */
const CODEX_BASELINE_TAG = "rust-v0.149.0";
const CODEX_MODELS_URL = `https://raw.githubusercontent.com/openai/codex/${CODEX_BASELINE_TAG}/codex-rs/models-manager/models.json`;
const MODELS_DEV_URL = "https://models.dev/catalog.json";
const OPENROUTER_URL = "https://openrouter.ai/api/v1/models";

/** 模板基底条目：取内置目录里最新一代的可见模型。 */
const TEMPLATE_BASE_SLUG = "gpt-5.6-sol";

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
 * 校验并规范化手写的第三方官方条目（条目形态即 model_catalog_json 的权威形状）。
 *
 * 不联网：GPT/codex 基线条目已改为应用启动时从本机 codex 导出（`codex debug models
 * --bundled`），本文件只维护其它厂商（如 deepseek）提供的条目。
 */
async function updateOfficial() {
  const parsed = JSON.parse(await readResource("official-models.json"));
  if (!Array.isArray(parsed?.models)) {
    throw new Error("official-models.json 缺少 models 数组");
  }

  const seen = new Set();
  const models = [];
  for (const model of parsed.models) {
    if (!model || typeof model !== "object") {
      throw new Error("official-models.json 存在非对象条目");
    }
    const slug = typeof model.slug === "string" ? model.slug.trim() : "";
    if (!slug) {
      throw new Error(
        "official-models.json 存在缺少 slug 的条目（是否把整个 models.json 当成单条粘进来了？）",
      );
    }
    const template = model.model_messages?.instructions_template;
    if (typeof template !== "string" || !template.trim()) {
      throw new Error(`official-models.json 条目 ${slug} 缺少 model_messages.instructions_template`);
    }
    if (seen.has(slug)) {
      console.warn(`警告：official-models.json 存在重复 slug ${slug}，已保留首条`);
      continue;
    }
    seen.add(slug);
    model.slug = slug;
    models.push(model);
  }
  if (models.length === 0) {
    throw new Error("official-models.json 没有任何条目");
  }

  console.log(`第三方官方条目：${models.length} 条（${models.map((m) => m.slug).join(", ")}）`);
  await writeResource("official-models.json", `${JSON.stringify({ models }, null, 2)}\n`);
}

/**
 * 由官方条目池里的一个条目推导基底模板：
 * 固定值全部固化在模板里，代码不持有任何固定取值；
 * 数据驱动字段只留占位值，生成时由字段源覆盖；其中 supported_reasoning_levels
 * 保留基底条目的档位与描述，作为渲染期的“档位描述表”（写入前必被覆盖，不会泄漏）。
 */
async function updateTemplate() {
  // 基底取自 codex 基线 models.json（不再从 official-models.json 派生：那里只有第三方条目）
  const text = await fetchJson(CODEX_MODELS_URL);
  const official = JSON.parse(text);
  if (!Array.isArray(official?.models) || official.models.length === 0) {
    throw new Error("codex 基线目录缺少 models 数组");
  }
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

/** 单个模型条目 → 精简字段版本（只留 Rust 侧映射需要的能力参数）。 */
function slimModel(model) {
  const keep = {};
  for (const key of MODEL_KEEP_KEYS) {
    if (key in model) {
      keep[key] = key === "description" ? truncate(model[key], DESCRIPTION_MAX) : model[key];
    }
  }
  return keep;
}

/**
 * models.dev 全量目录（catalog.json：`{ models, providers }`）→ 精简字段版本。
 *
 * 形态与远端一致：`providers` 保留全部 provider 分组（含中转/聚合商），`models` 保留
 * 模型级规范条目；Rust 侧只看模型 ID，不按 base_url 定位分组，因此精简掉 `api` / `name`。
 */
async function updateModelsDev() {
  const text = await fetchJson(MODELS_DEV_URL);
  const parsed = JSON.parse(text);
  const providers = {};
  const canonical = {};
  let providerModels = 0;
  let canonicalModels = 0;

  for (const [providerId, provider] of Object.entries(parsed.providers ?? {})) {
    if (!provider || typeof provider !== "object" || !provider.models) continue;
    const slimModels = {};
    for (const [modelId, model] of Object.entries(provider.models)) {
      if (!model || typeof model !== "object") continue;
      slimModels[modelId] = slimModel(model);
      providerModels += 1;
    }
    if (Object.keys(slimModels).length === 0) continue;
    providers[providerId] = { models: slimModels };
  }

  for (const [modelId, model] of Object.entries(parsed.models ?? {})) {
    if (!model || typeof model !== "object") continue;
    canonical[modelId] = slimModel(model);
    canonicalModels += 1;
  }

  if (providerModels === 0) throw new Error("models.dev 目录没有可用的 provider 分组");
  if (canonicalModels === 0) throw new Error("models.dev 目录没有可用的模型条目");
  console.log(
    `models.dev 全量精简：${Object.keys(providers).length} 个 provider / ${providerModels} 个模型，` +
      `模型清单 ${canonicalModels} 条`,
  );
  await writeResource("models-dev.json", `${JSON.stringify({ models: canonical, providers })}\n`);
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
