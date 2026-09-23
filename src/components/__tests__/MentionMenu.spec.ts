import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";

vi.mock("@tauri-apps/api/core", () => ({
  invoke: vi.fn(),
  convertFileSrc: (p: string) => "asset://mock/" + p,
}));

vi.mock("../../composables/useCodex", async (importOriginal) => {
  const mod =
    await importOriginal<typeof import("../../composables/useCodex")>();
  return { ...mod, ensureSkills: vi.fn() };
});

import MentionMenu from "../MentionMenu.vue";
import { ensureSkills } from "../../composables/useCodex";
import type { PluginItem, SessionTab } from "../../composables/useCodex";
import { makeSessionTab } from "../../composables/__tests__/useCodexTestHarness";
import type { FuzzyFileResult } from "../../lib/mention";

const mockedEnsureSkills = vi.mocked(ensureSkills);

const FILE_RESULT: FuzzyFileResult = {
  root: "C:/repo",
  path: "src/a/search.txt",
  match_type: "file",
  file_name: "search.txt",
  score: 1,
  indices: null,
};

function makePlugin(over: Partial<PluginItem> = {}): PluginItem {
  return {
    id: "p1",
    name: "plugin-alpha",
    displayName: "Alpha Plugin",
    description: "用于搜索的插件",
    path: "C:/p",
    iconPath: "",
    iconUrl: "",
    brandColor: "#ff0000",
    ...over,
  };
}

let tab: SessionTab;

describe("MentionMenu @/$ 联合菜单", () => {
  beforeEach(() => {
    tab = makeSessionTab("s1", "t1", {
      plugins: { plugins: [makePlugin()], loaded: true },
      skills: { skills: [], loaded: true },
    });
    mockedEnsureSkills.mockClear();
  });

  it("@ 分支：固定「选择文件/文件夹」行在插件之前，插件全部展示", () => {
    const w = mount(MentionMenu, {
      props: { tab, kind: "@", token: "", results: [], searching: false },
    });
    const labels = w.findAll(".menu-item-label").map((n) => n.text());
    expect(labels[0]).toBe("选择文件…");
    expect(labels[1]).toBe("选择文件夹…");
    expect(labels).toContain("Alpha Plugin");
    expect(w.text()).not.toContain("暂无可用插件");
  });

  it("@ 分支：有 token 时命中插件排在前、命中文件在后，组标题正确", () => {
    tab.plugins = {
      plugins: [makePlugin({ description: "searchable plugin" })],
      loaded: true,
    };
    const w = mount(MentionMenu, {
      props: {
        tab,
        kind: "@",
        token: "search",
        results: [FILE_RESULT],
        searching: false,
      },
    });
    const labels = w.findAll(".menu-item-label").map((n) => n.text());
    expect(labels[0]).toBe("选择文件…");
    expect(labels[1]).toBe("选择文件夹…");
    expect(labels.indexOf("Alpha Plugin")).toBeGreaterThan(-1);
    expect(labels.indexOf("search.txt")).toBeGreaterThan(
      labels.indexOf("Alpha Plugin"),
    );
    expect(w.text()).toContain("插件");
    expect(w.text()).toContain("文件");
  });

  it("@ 分支：点击文件行发出 mention 附件（路径转正斜杠）", async () => {
    const w = mount(MentionMenu, {
      props: {
        tab,
        kind: "@",
        token: "search",
        results: [FILE_RESULT],
        searching: false,
      },
    });
    const fileBtn = w
      .findAll(".menu-item")
      .find((b) => b.text().includes("search.txt"));
    expect(fileBtn).toBeTruthy();
    await fileBtn!.trigger("click");
    const emitted = w.emitted("select-file");
    expect(emitted?.[0]?.[0]).toEqual({
      type: "mention",
      name: "search.txt",
      path: "C:/repo/src/a/search.txt",
    });
  });

  it("@ 分支：点击固定行分别发出 pick-files / pick-dir", async () => {
    const w = mount(MentionMenu, {
      props: { tab, kind: "@", token: "", results: [], searching: false },
    });
    await w.findAll(".menu-item")[0].trigger("click");
    await w.findAll(".menu-item")[1].trigger("click");
    expect(w.emitted("pick-files")).toBeTruthy();
    expect(w.emitted("pick-dir")).toBeTruthy();
  });

  it("@ 分支：插件无图标时渲染品牌色首字母占位", () => {
    const w = mount(MentionMenu, {
      props: { tab, kind: "@", token: "", results: [], searching: false },
    });
    const fallback = w.find(".plugin-icon-fallback");
    expect(fallback.exists()).toBe(true);
    expect(fallback.text()).toBe("A");
    expect(fallback.attributes("style")).toContain("#ff0000");
  });

  it("$ 分支：渲染技能列表并可点击发出 skill 附件", async () => {
    tab.skills = {
      skills: [
        {
          name: "SkillA",
          key: "skill-a",
          path: "C:/s/SKILL.md",
          desc: "描述",
          shortDesc: "短描述",
        },
      ],
      loaded: true,
    };
    const w = mount(MentionMenu, {
      props: { tab, kind: "$", token: "", results: [], searching: false },
    });
    await flushPromises();
    expect(mockedEnsureSkills).toHaveBeenCalled();
    expect(w.text()).toContain("SkillA");
    await w.find(".menu-item").trigger("click");
    expect(w.emitted("select-file")?.[0]?.[0]).toEqual({
      type: "skill",
      name: "skill-a",
      path: "C:/s/SKILL.md",
      source: "skill",
    });
  });

  it("$ 分支：token 按名称/key 过滤技能", () => {
    tab.skills = {
      skills: [
        {
          name: "SkillA",
          key: "skill-a",
          path: "C:/s/SKILL.md",
          desc: "",
          shortDesc: "",
        },
        {
          name: "Other",
          key: "other",
          path: "C:/o/SKILL.md",
          desc: "",
          shortDesc: "",
        },
      ],
      loaded: true,
    };
    const w = mount(MentionMenu, {
      props: { tab, kind: "$", token: "skill", results: [], searching: false },
    });
    expect(w.text()).toContain("SkillA");
    expect(w.text()).not.toContain("Other");
  });
});
