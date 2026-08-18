import { describe, expect, it } from "vitest";
import { Compartment } from "@codemirror/state";
import {
  buildEditorExtensions,
  createEditorState,
  editorTheme,
  editorThemeSpec,
  languageForPath,
} from "../editorSetup";

describe("languageForPath", () => {
  it("常见代码扩展名返回语言扩展", () => {
    expect(languageForPath("D:\\repo\\src\\a.ts")).not.toBeNull();
    expect(languageForPath("src/a.tsx")).not.toBeNull();
    expect(languageForPath("a.py")).not.toBeNull();
    expect(languageForPath("a.rs")).not.toBeNull();
    expect(languageForPath("a.cs")).not.toBeNull();
    expect(languageForPath("a.cpp")).not.toBeNull();
    expect(languageForPath("a.go")).not.toBeNull();
    expect(languageForPath("a.java")).not.toBeNull();
    expect(languageForPath("a.kt")).not.toBeNull();
    expect(languageForPath("a.json")).not.toBeNull();
    expect(languageForPath("a.md")).not.toBeNull();
    expect(languageForPath("a.yaml")).not.toBeNull();
    expect(languageForPath("a.sh")).not.toBeNull();
    expect(languageForPath("a.ps1")).not.toBeNull();
    expect(languageForPath("a.sql")).not.toBeNull();
    expect(languageForPath("a.css")).not.toBeNull();
    expect(languageForPath("a.html")).not.toBeNull();
    expect(languageForPath("a.xml")).not.toBeNull();
    expect(languageForPath("a.ini")).not.toBeNull();
    expect(languageForPath("a.php")).not.toBeNull();
    expect(languageForPath("a.rb")).not.toBeNull();
    expect(languageForPath("a.diff")).not.toBeNull();
  });

  it("未知扩展名或无扩展名返回 null（纯文本）", () => {
    expect(languageForPath("a.txt")).toBeNull();
    expect(languageForPath("noext")).toBeNull();
    expect(languageForPath(".gitignore")).toBeNull();
    expect(languageForPath("")).toBeNull();
  });
});

describe("buildEditorExtensions", () => {
  it("组装后可创建编辑器状态", () => {
    const savedText = () => null;
    const ext = buildEditorExtensions({
      language: languageForPath("a.ts"),
      readOnly: false,
      wrap: false,
      wrapCompartment: new Compartment(),
      savedText,
      onDirtyChange: () => {},
      onCursorChange: () => {},
      onSave: () => {},
    });
    expect(ext.length).toBeGreaterThan(0);
    const state = createEditorState("const a = 1;\n", ext);
    expect(state.doc.toString()).toBe("const a = 1;\n");
  });

  it("editorTheme 存在", () => {
    expect(editorTheme).toBeTruthy();
  });
});

describe("editorTheme 查找面板按钮", () => {
  it("按钮不叠加深色渐变（浅色主题黑字黑底修复）", () => {
    const btn = editorThemeSpec[".cm-panels .cm-button"];
    expect(btn.backgroundColor).toBe("var(--bg-active)");
    expect(btn.backgroundImage).toBe("none");
    expect(btn.color).toBe("var(--text-bright)");
  });
});
