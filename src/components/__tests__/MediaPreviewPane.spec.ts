import { describe, expect, it } from "vitest";
import { mount } from "@vue/test-utils";
import MediaPreviewPane from "../MediaPreviewPane.vue";
import type { PreviewEditorTab } from "../../composables/useEditorTabs";

function makeTab(over: Partial<PreviewEditorTab> = {}): PreviewEditorTab {
  return {
    kind: "preview",
    icon: "file",
    previewType: "video",
    id: "p1",
    workspace: "D:/repo",
    path: "media/clip.mp4",
    title: "clip.mp4",
    loading: false,
    error: "",
    imageUrl: "",
    mediaUrl: "asset://D:/repo/media/clip.mp4",
    pdfData: null,
    xlsxData: null,
    docxData: null,
    pptxData: null,
    xlsxSheetIndex: 0,
    pageCount: null,
    stale: false,
    ...over,
  };
}

describe("MediaPreviewPane 音视频播放", () => {
  it("视频：渲染带 controls 的 video，src 为 mediaUrl", () => {
    const w = mount(MediaPreviewPane, {
      props: {
        tab: makeTab({
          previewType: "video",
          mediaUrl: "asset://D:/repo/media/clip.mp4",
        }),
      },
    });
    const video = w.find("video");
    expect(video.exists()).toBe(true);
    expect(video.attributes("src")).toBe("asset://D:/repo/media/clip.mp4");
    expect(video.attributes("controls")).toBeDefined();
    expect(w.find("audio").exists()).toBe(false);
  });

  it("音频：渲染带 controls 的 audio，src 为 mediaUrl", () => {
    const w = mount(MediaPreviewPane, {
      props: {
        tab: makeTab({
          previewType: "audio",
          mediaUrl: "asset://D:/repo/media/song.mp3",
        }),
      },
    });
    const audio = w.find("audio");
    expect(audio.exists()).toBe(true);
    expect(audio.attributes("src")).toBe("asset://D:/repo/media/song.mp3");
    expect(audio.attributes("controls")).toBeDefined();
    expect(w.find("video").exists()).toBe(false);
  });

  it("媒体加载失败时显示中文错误提示", async () => {
    const w = mount(MediaPreviewPane, {
      props: {
        tab: makeTab({
          previewType: "video",
          mediaUrl: "asset://D:/repo/media/bad.mp4",
        }),
      },
    });
    await w.find("video").trigger("error");
    expect(w.text()).toContain("无法播放该文件");
  });
});
