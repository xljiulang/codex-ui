import { beforeEach, describe, expect, it, vi } from "vitest";
import { ref } from "vue";
import { flushPromises } from "@vue/test-utils";
import type { UserInput } from "../../lib/types";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn() }));
vi.mock("../useCodex", async (importOriginal) => {
  const mod = await importOriginal<typeof import("../useCodex")>();
  return { ...mod, setToast: vi.fn() };
});

import { invoke } from "@tauri-apps/api/core";
import { setToast } from "../useCodex";
import { useComposerAttachments } from "../useComposerAttachments";
import { findDropTarget } from "../dropTargets";

const mockedInvoke = vi.mocked(invoke);
const mockedToast = vi.mocked(setToast);

function setup() {
  const rowAttachments = ref<UserInput[]>([]);
  const syncAttachments = vi.fn();
  const rootRef = ref<HTMLElement | null>(null);
  const att = useComposerAttachments({ rowAttachments, syncAttachments, rootRef });
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

  it("registerTarget 后 drop 事件派发走 addDroppedPaths 进附件区", async () => {
    const rowAttachments = ref<UserInput[]>([]);
    const syncAttachments = vi.fn();
    const rootRef = ref<HTMLElement | null>(null);
    const att = useComposerAttachments({
      rowAttachments,
      syncAttachments,
      rootRef,
    });

    const el = {
      getBoundingClientRect: () => ({
        left: 0,
        top: 0,
        right: 400,
        bottom: 200,
        width: 400,
        height: 200,
        x: 0,
        y: 0,
        toJSON: () => ({}),
      }),
    } as unknown as HTMLElement;
    rootRef.value = el;
    att.registerTarget();

    // 全局命中后触发目标回调（等价 useGlobalDragDrop 的坐标命中派发）
    const hit = findDropTarget(100, 100);
    expect(hit).not.toBeNull();
    hit!.onDropPaths(["D:\\x\\a.ts", "D:\\x\\b.pdf"]);
    await flushPromises();

    // mention 附件路径被 toUserAttachment 转成正斜杠；localImage 保留原样
    // （addDroppedPaths 只产生 mention/localImage，不存在 text/skill 项）
    expect(
      rowAttachments.value
        .filter((a): a is UserInput & { path: string } => "path" in a)
        .map((a) => a.path),
    ).toEqual([
      "D:/x/a.ts",
      "D:/x/b.pdf",
    ]);
    expect(syncAttachments).toHaveBeenCalled();

    // 清理：注销避免影响后续用例
    att.unregisterTarget();
    expect(findDropTarget(100, 100)).toBeNull();
  });

  it("registerTarget 的 setDragging 驱动高亮", () => {
    const rowAttachments = ref<UserInput[]>([]);
    const rootRef = ref<HTMLElement | null>(null);
    const att = useComposerAttachments({
      rowAttachments,
      syncAttachments: vi.fn(),
      rootRef,
    });

    const el = {
      getBoundingClientRect: () => ({
        left: 0,
        top: 0,
        right: 400,
        bottom: 200,
        width: 400,
        height: 200,
        x: 0,
        y: 0,
        toJSON: () => ({}),
      }),
    } as unknown as HTMLElement;
    rootRef.value = el;
    att.registerTarget();

    // 模拟全局 enter/over 推送
    expect(att.dragging.value).toBe(false);
    const hit = findDropTarget(100, 100);
    hit!.setDragging(true);
    expect(att.dragging.value).toBe(true);

    att.unregisterTarget();
  });
});
