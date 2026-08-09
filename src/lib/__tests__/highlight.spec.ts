import { describe, expect, it } from "vitest";
import { languageFromPath } from "../highlight";

describe("languageFromPath", () => {
  it("常见扩展名映射到已注册语言", () => {
    expect(languageFromPath("D:\\repo\\src\\a.ts")).toBe("typescript");
    expect(languageFromPath("src/a.py")).toBe("python");
    expect(languageFromPath("a.rs")).toBe("rust");
    expect(languageFromPath("a.cs")).toBe("csharp");
    expect(languageFromPath("README.md")).toBe("markdown");
    expect(languageFromPath("a.json")).toBe("json");
    expect(languageFromPath("a.yml")).toBe("yaml");
  });

  it("未知扩展名或无扩展名返回 null", () => {
    expect(languageFromPath("a.txt")).toBeNull();
    expect(languageFromPath("noext")).toBeNull();
    expect(languageFromPath(".gitignore")).toBeNull();
    expect(languageFromPath("")).toBeNull();
  });
});
