import { describe, expect, it } from "vitest";
import { list, obj, permText, str } from "../interaction";

describe("str", () => {
  it("null/undefined 返回空串，其余转字符串", () => {
    expect(str(null)).toBe("");
    expect(str(undefined)).toBe("");
    expect(str("abc")).toBe("abc");
    expect(str(0)).toBe("0");
    expect(str(false)).toBe("false");
  });
});

describe("list", () => {
  it("数组原样返回，非数组返回空数组", () => {
    const a = [1, 2];
    expect(list(a)).toBe(a);
    expect(list(null)).toEqual([]);
    expect(list("x")).toEqual([]);
  });
});

describe("obj", () => {
  it("对象返回 Record，非对象返回空对象", () => {
    const o = { a: 1 };
    expect(obj(o)).toBe(o);
    expect(obj(null)).toEqual({});
    expect(obj("x")).toEqual({});
  });
});

describe("permText", () => {
  it("标量直接转文本", () => {
    expect(permText(null)).toBe("");
    expect(permText("read")).toBe("read");
    expect(permText(3)).toBe("3");
    expect(permText(true)).toBe("true");
  });

  it("数组用分号连接并递归", () => {
    expect(permText(["read", "write"])).toBe("read；write");
    expect(permText([["a"], "b"])).toBe("a；b");
  });

  it("对象渲染为 键：值 并用分号连接", () => {
    expect(permText({ read: true, dirs: ["src"] })).toBe(
      "read：true；dirs：src",
    );
  });
});
