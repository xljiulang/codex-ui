import { beforeEach, describe, expect, it, vi } from "vitest";
import { flushPromises, mount } from "@vue/test-utils";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("../../composables/useCodex", async (importOriginal) => {
  const mod =
    await importOriginal<typeof import("../../composables/useCodex")>();
  return { ...mod, askConfirm: vi.fn(), setToast: vi.fn() };
});

import { invoke } from "@tauri-apps/api/core";
import { askConfirm } from "../../composables/useCodex";
import GitRemotesSection from "../GitRemotesSection.vue";

const mockedInvoke = vi.mocked(invoke);
const mockedAskConfirm = vi.mocked(askConfirm);

const originRemote = {
  name: "origin",
  fetchUrl: "https://github.com/x/y.git",
  pushUrl: "https://github.com/x/y.git",
};
const upstreamRemote = {
  name: "upstream",
  fetchUrl: "https://example.com/u.git",
  pushUrl: null,
};

function mountSection(current: string | null = "origin") {
  return mount(GitRemotesSection, {
    props: { workspace: "D:\\repo", currentRemote: current },
  });
}

describe("GitRemotesSection", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
    mockedAskConfirm.mockReset();
  });

  it("挂载时加载远端并渲染行与当前徽章，回传 currentRemote", async () => {
    mockedInvoke.mockResolvedValueOnce({
      current: "origin",
      remotes: [originRemote, upstreamRemote],
    });
    const wrapper = mountSection();
    await flushPromises();

    expect(mockedInvoke).toHaveBeenCalledWith("git_changes_remotes", {
      workspace: "D:\\repo",
    });
    const rows = wrapper.findAll(".git-remote-row");
    expect(rows).toHaveLength(2);
    expect(rows[0].find(".git-remote-name").text()).toContain("origin");
    expect(rows[0].find(".git-remote-badge").exists()).toBe(true);
    expect(rows[1].find(".git-remote-url").text()).toContain(
      "example.com/u.git",
    );
    expect(wrapper.emitted("update:currentRemote")?.[0]).toEqual(["origin"]);
    wrapper.unmount();
  });

  it("无远端时显示空态", async () => {
    mockedInvoke.mockResolvedValueOnce({ current: null, remotes: [] });
    const wrapper = mountSection(null);
    await flushPromises();
    expect(wrapper.find(".git-remote-branch-empty").text()).toBe("暂无远端");
    wrapper.unmount();
  });

  it("添加远端：调用 remote_add、回传并清空输入", async () => {
    mockedInvoke
      .mockResolvedValueOnce({ current: null, remotes: [] })
      .mockResolvedValueOnce({
        current: "upstream",
        remotes: [originRemote, upstreamRemote],
      });
    const wrapper = mountSection(null);
    await flushPromises();

    const inputs = wrapper.findAll(".git-remote-add input");
    await inputs[0].setValue("upstream");
    await inputs[1].setValue("https://example.com/u.git");
    await wrapper.find(".git-remote-add-btn").trigger("click");
    await flushPromises();

    expect(mockedInvoke).toHaveBeenCalledWith("git_changes_remote_add", {
      workspace: "D:\\repo",
      name: "upstream",
      url: "https://example.com/u.git",
    });
    expect(wrapper.emitted("update:currentRemote")?.[1]).toEqual(["upstream"]);
    expect((inputs[0].element as HTMLInputElement).value).toBe("");
    wrapper.unmount();
  });

  it("切换上游：调用 switch_upstream 并回传", async () => {
    mockedInvoke
      .mockResolvedValueOnce({
        current: "origin",
        remotes: [originRemote, upstreamRemote],
      })
      .mockResolvedValueOnce({
        current: "upstream",
        remotes: [originRemote, upstreamRemote],
      });
    const wrapper = mountSection();
    await flushPromises();

    const upstreamRow = wrapper
      .findAll(".git-remote-row")
      .find((r) => r.find(".git-remote-name").text().includes("upstream"))!;
    await upstreamRow.find(".git-remote-switch").trigger("click");
    await flushPromises();

    expect(mockedInvoke).toHaveBeenCalledWith(
      "git_changes_remote_switch_upstream",
      { workspace: "D:\\repo", remote: "upstream" },
    );
    expect(wrapper.emitted("update:currentRemote")?.[1]).toEqual(["upstream"]);
    wrapper.unmount();
  });

  it("删除远端：未确认不调用，确认后调用 remote_remove 并回传", async () => {
    mockedInvoke
      .mockResolvedValueOnce({
        current: "origin",
        remotes: [originRemote],
      })
      .mockResolvedValueOnce({ current: null, remotes: [] });
    const wrapper = mountSection();
    await flushPromises();

    mockedAskConfirm.mockResolvedValueOnce(false);
    await wrapper.find(".git-remote-delete").trigger("click");
    await flushPromises();
    expect(
      mockedInvoke.mock.calls.some(
        ([cmd]) => cmd === "git_changes_remote_remove",
      ),
    ).toBe(false);

    mockedAskConfirm.mockResolvedValueOnce(true);
    await wrapper.find(".git-remote-delete").trigger("click");
    await flushPromises();
    expect(mockedInvoke).toHaveBeenCalledWith("git_changes_remote_remove", {
      workspace: "D:\\repo",
      name: "origin",
    });
    expect(wrapper.emitted("update:currentRemote")?.[1]).toEqual([null]);
    wrapper.unmount();
  });
});
