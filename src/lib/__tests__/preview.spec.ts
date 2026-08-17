import { describe, expect, it } from "vitest";
import { base64ToBytes, extOf, previewTypeForName } from "../preview";

describe("previewTypeForName 扩展名识别", () => {
  it("pdf 扩展名（大小写不敏感）", () => {
    expect(previewTypeForName("a.pdf")).toBe("pdf");
    expect(previewTypeForName("A.PDF")).toBe("pdf");
    expect(previewTypeForName("dir/a.Pdf")).toBe("pdf");
  });

  it("常见图像扩展名（大小写不敏感）", () => {
    const exts = [
      "png",
      "jpg",
      "jpeg",
      "gif",
      "webp",
      "bmp",
      "svg",
      "ico",
      "avif",
    ];
    for (const ext of exts) {
      expect(previewTypeForName(`pic.${ext}`)).toBe("image");
      expect(previewTypeForName(`pic.${ext.toUpperCase()}`)).toBe("image");
    }
  });

  it("xlsx 扩展名（大小写不敏感），xls 不命中", () => {
    expect(previewTypeForName("a.xlsx")).toBe("xlsx");
    expect(previewTypeForName("DIR/report.XLSX")).toBe("xlsx");
    expect(previewTypeForName("a.Xlsx")).toBe("xlsx");
    expect(previewTypeForName("a.xls")).toBeNull();
    expect(previewTypeForName("a.xlsm")).toBeNull();
  });

  it("其余类型返回 null", () => {
    expect(previewTypeForName("a.txt")).toBeNull();
    expect(previewTypeForName("Makefile")).toBeNull();
    expect(previewTypeForName("a.pdf.bak")).toBeNull();
    expect(previewTypeForName("a.")).toBeNull();
  });
});

describe("extOf 边界", () => {
  it("取最后一个点后的扩展名", () => {
    expect(extOf("a.tar.gz")).toBe("gz");
    expect(extOf("README.md")).toBe("md");
  });

  it("无扩展名与点文件返回 null", () => {
    expect(extOf("noext")).toBeNull();
    expect(extOf(".gitignore")).toBeNull();
  });
});

describe("base64ToBytes", () => {
  it("base64 解码为 Uint8Array", () => {
    const bytes = base64ToBytes("aGVsbG8=");
    expect(Array.from(bytes)).toEqual([104, 101, 108, 108, 111]);
  });
});
