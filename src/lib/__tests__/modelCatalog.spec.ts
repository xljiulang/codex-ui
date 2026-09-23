import { describe, expect, it } from "vitest";
import { filterCatalogByModelIds } from "../modelCatalog";

function catalogText(): string {
  return JSON.stringify(
    {
      models: [
        { slug: "deepseek-chat", display_name: "Deepseek-Chat", priority: 1 },
        {
          slug: "deepseek-reasoner",
          display_name: "Deepseek-Reasoner",
          priority: 2,
        },
        { slug: "deepseek-coder", display_name: "Deepseek-Coder", priority: 3 },
      ],
    },
    null,
    2,
  );
}

describe("filterCatalogByModelIds", () => {
  it("只保留选中的模型并按原顺序重排 priority", () => {
    const result = JSON.parse(
      filterCatalogByModelIds(catalogText(), [
        "deepseek-coder",
        "deepseek-chat",
      ]),
    ) as {
      models: Array<{ slug: string; priority: number }>;
    };
    expect(result.models.map((model) => model.slug)).toEqual([
      "deepseek-chat",
      "deepseek-coder",
    ]);
    expect(result.models.map((model) => model.priority)).toEqual([1, 2]);
  });

  it("支持用 id 回退识别模型条目", () => {
    const result = JSON.parse(
      filterCatalogByModelIds(
        JSON.stringify({
          models: [{ id: "model-a", display_name: "Model-A", priority: 9 }],
        }),
        ["model-a"],
      ),
    ) as {
      models: Array<{ id: string; priority: number }>;
    };
    expect(result.models).toEqual([
      { id: "model-a", display_name: "Model-A", priority: 1 },
    ]);
  });

  it("目录结构非法时抛错", () => {
    expect(() => filterCatalogByModelIds('{"models":{}}', ["a"])).toThrow(
      "模型目录缺少 models 数组",
    );
  });
});
