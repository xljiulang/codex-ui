import { beforeEach, describe, expect, it, vi } from "vitest";
import { ref } from "vue";
import { flushPromises } from "@vue/test-utils";
import type { UserInput } from "../../lib/types";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("@tauri-apps/api/webview", () => ({
  getCurrentWebview: vi.fn(),
}));
vi.mock("../useCodex", async (importOriginal) => {
  const mod = await importOriginal<typeof import("../useCodex")>();
  return { ...mod, setToast: vi.fn() };
});

import { invoke } from "@tauri-apps/api/core";
import { setToast } from "../useCodex";
import { useComposerAttachments } from "../useComposerAttachments";

const mockedInvoke = vi.mocked(invoke);
const mockedToast = vi.mocked(setToast);

function setup() {
  const rowAttachments = ref<UserInput[]>([]);
  const syncAttachments = vi.fn();
  const att = useComposerAttachments({ rowAttachments, syncAttachments });
  return { rowAttachments, syncAttachments, att };
}

function pasteEvent(files: File[]): ClipboardEvent {
  return {
    clipboardData: {
      items: files.map((f) => ({
        kind: "file",
        getAsFile: () => f,
      })),
    },
    preventDefault: vi.fn(),
  } as unknown as ClipboardEvent;
}

function file(name: string, type: string): File {
  return {
    name,
    type,
    size: 10,
    arrayBuffer: vi.fn().mockResolvedValue(new ArrayBuffer(3)),
  } as unknown as File;
}

describe("useComposerAttachments 粘贴", () => {
  beforeEach(() => {
    mockedInvoke.mockReset();
    mockedToast.mockClear();
  });

  it("无文件项时放行默认粘贴", () => {
    const { att } = setup();
    expect(att.handlePasteDom(pasteEvent([]))).toBe(false);
  });

  it("图片带原始路径 → localImage 附件，不落盘", async () => {
    const { rowAttachments, syncAttachments, att } = setup();
    mockedInvoke.mockResolvedValueOnce(["D:\\x\\a.png"]);
    expect(att.handlePasteDom(pasteEvent([file("a.png", "image/png")]))).toBe(
      true,
    );
    await flushPromises();
    expect(rowAttachments.value).toEqual([
      { type: "localImage", path: "D:\\x\\a.png" },
    ]);
    expect(syncAttachments).toHaveBeenCalled();
    expect(mockedInvoke).not.toHaveBeenCalledWith(
      "save_pasted_image",
      expect.anything(),
    );
  });

  it("无原始路径的图片 → save_pasted_image 落盘后加入附件", async () => {
    const { rowAttachments, att } = setup();
    mockedInvoke
      .mockResolvedValueOnce([]) // clipboard_file_paths 为空
      .mockResolvedValueOnce("D:\\tmp\\pasted.png"); // save_pasted_image
    att.handlePasteDom(pasteEvent([file("pasted", "image/png")]));
    await flushPromises();
    expect(mockedInvoke).toHaveBeenCalledWith("save_pasted_image", {
      bytes: expect.any(Array),
      name: "pasted.png",
    });
    expect(rowAttachments.value).toEqual([
      { type: "localImage", path: "D:\\tmp\\pasted.png" },
    ]);
  });

  it("非图片无原始路径 → 提示暂不支持", async () => {
    const { rowAttachments, att } = setup();
    mockedInvoke.mockResolvedValueOnce([]);
    att.handlePasteDom(pasteEvent([file("a.pdf", "application/pdf")]));
    await flushPromises();
    expect(mockedToast).toHaveBeenCalledWith(
      expect.stringContaining("暂不支持该粘贴"),
    );
    expect(rowAttachments.value).toEqual([]);
  });

  it("文本文件带原始路径 → mention 附件", async () => {
    const { rowAttachments, att } = setup();
    mockedInvoke.mockResolvedValueOnce(["D:\\x\\a.txt"]);
    att.handlePasteDom(pasteEvent([file("a.txt", "text/plain")]));
    await flushPromises();
    expect(rowAttachments.value[0]).toMatchObject({
      type: "mention",
      name: "a.txt",
    });
  });
});
