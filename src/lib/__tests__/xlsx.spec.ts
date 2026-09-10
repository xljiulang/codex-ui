import { describe, expect, it } from "vitest";
import * as XLSX from "xlsx";
import { parseXlsx } from "../xlsx";

/** 内存构造工作簿 → xlsx 字节（parseXlsx 的输入） */
function workbookBytes(wb: XLSX.WorkBook): Uint8Array {
  const out = XLSX.write(wb, { type: "array", bookType: "xlsx" });
  return new Uint8Array(out as ArrayBuffer);
}

/** 指定格式的工作簿字节（xlsb/biff8/ods 等） */
function bookBytes(wb: XLSX.WorkBook, bookType: XLSX.BookType): Uint8Array {
  const out = XLSX.write(wb, { type: "array", bookType });
  return new Uint8Array(out as ArrayBuffer);
}

/** UTF-16LE + BOM 字节（模拟 Excel「Unicode 文本」导出） */
function utf16leBytes(text: string): Uint8Array {
  const bytes = new Uint8Array(2 + text.length * 2);
  bytes[0] = 0xff;
  bytes[1] = 0xfe;
  for (let i = 0; i < text.length; i += 1) {
    const code = text.charCodeAt(i);
    bytes[2 + i * 2] = code & 0xff;
    bytes[3 + i * 2] = code >> 8;
  }
  return bytes;
}

function simpleWorkbook(): XLSX.WorkBook {
  const wb = XLSX.utils.book_new();
  XLSX.utils.book_append_sheet(
    wb,
    XLSX.utils.aoa_to_sheet([
      ["名称", "数量"],
      ["苹果", 3],
    ]),
    "数据",
  );
  return wb;
}

describe("parseXlsx 工作簿解析", () => {
  it("多工作表：返回工作表名与逐表网格", () => {
    const wb = XLSX.utils.book_new();
    XLSX.utils.book_append_sheet(
      wb,
      XLSX.utils.aoa_to_sheet([
        ["名称", "数量", "日期"],
        ["苹果", 3, new Date(2024, 0, 15)],
      ]),
      "数据",
    );
    XLSX.utils.book_append_sheet(wb, XLSX.utils.aoa_to_sheet([]), "空表");

    const parsed = parseXlsx(workbookBytes(wb));
    expect(parsed.sheetNames).toEqual(["数据", "空表"]);
    expect(parsed.sheets).toHaveLength(2);

    const data = parsed.sheets[0];
    expect(data.name).toBe("数据");
    expect(data.rowCount).toBe(2);
    expect(data.colCount).toBe(3);
    expect(data.rows[0]).toEqual(["名称", "数量", "日期"]);
    // raw:false 取格式化文本：数字与日期均为字符串且行补齐到表宽
    expect(data.rows[1][0]).toBe("苹果");
    expect(data.rows[1][1]).toBe("3");
    expect(typeof data.rows[1][2]).toBe("string");
    expect(data.rows[1][2]).toContain("15");
    expect(data.rows[1]).toHaveLength(3);
  });

  it("合并单元格：仅左上角显示值，覆盖单元格为空串", () => {
    const ws = XLSX.utils.aoa_to_sheet([
      ["标题", "b"],
      ["c", "d"],
    ]);
    ws["!merges"] = [{ s: { r: 0, c: 0 }, e: { r: 0, c: 1 } }];
    const wb = XLSX.utils.book_new();
    XLSX.utils.book_append_sheet(wb, ws, "S1");

    const sheet = parseXlsx(workbookBytes(wb)).sheets[0];
    expect(sheet.rows[0][0]).toBe("标题");
    expect(sheet.rows[0][1]).toBe("");
    expect(sheet.rows[1][0]).toBe("c");
  });

  it("列宽：按 !cols 的 wch 换算并夹紧，缺省 120", () => {
    const ws = XLSX.utils.aoa_to_sheet([["a", "b", "c"]]);
    ws["!cols"] = [{ wch: 10 }, { wch: 100 }, { width: 1000 }];
    const wb = XLSX.utils.book_new();
    XLSX.utils.book_append_sheet(wb, ws, "S1");

    const widths = parseXlsx(workbookBytes(wb)).sheets[0].colWidths;
    expect(widths).toHaveLength(3);
    expect(widths[0]).toBe(Math.round(10 * 7) + 12); // 82
    expect(widths[1]).toBe(320); // wch 100 超上限夹紧
    expect(widths[2]).toBe(320); // width 1000 超上限夹紧
  });

  it("空工作表：无 !ref 时 rowCount/colCount 为 0", () => {
    const wb = XLSX.utils.book_new();
    XLSX.utils.book_append_sheet(wb, XLSX.utils.aoa_to_sheet([]), "空表");

    const sheet = parseXlsx(workbookBytes(wb)).sheets[0];
    expect(sheet.rowCount).toBe(0);
    expect(sheet.colCount).toBe(0);
    expect(sheet.rows).toEqual([]);
  });

  it("非法字节：抛出解析错误", () => {
    expect(() => parseXlsx(new Uint8Array([1, 2, 3, 4]))).toThrow();
  });

  it("xls（BIFF8，CFB 容器）：按扩展名解析", () => {
    const parsed = parseXlsx(bookBytes(simpleWorkbook(), "biff8"), "xls");
    expect(parsed.sheetNames).toEqual(["数据"]);
    expect(parsed.sheets[0].rows[0]).toEqual(["名称", "数量"]);
    expect(parsed.sheets[0].rows[1]).toEqual(["苹果", "3"]);
  });

  it("xlsb 与 ods（ZIP 容器家族）：按扩展名解析", () => {
    for (const [ext, bookType] of [
      ["xlsb", "xlsb"],
      ["ods", "ods"],
    ] as const) {
      const parsed = parseXlsx(bookBytes(simpleWorkbook(), bookType), ext);
      expect(parsed.sheets[0].rows[1], ext).toEqual(["苹果", "3"]);
    }
  });

  it("csv（无 BOM 的 UTF-8 中文）：走文本路径解码，不出现乱码", () => {
    const bytes = new TextEncoder().encode("名称,数量\n苹果,3");
    const sheet = parseXlsx(bytes, "csv").sheets[0];
    expect(sheet.rows[0]).toEqual(["名称", "数量"]);
    expect(sheet.rows[1]).toEqual(["苹果", "3"]);
  });

  it("tsv：按制表符分列", () => {
    const bytes = new TextEncoder().encode("名称\t数量\n苹果\t3");
    const sheet = parseXlsx(bytes, "tsv").sheets[0];
    expect(sheet.colCount).toBe(2);
    expect(sheet.rows[0]).toEqual(["名称", "数量"]);
    expect(sheet.rows[1]).toEqual(["苹果", "3"]);
  });

  it("UTF-16LE 文本（Excel 导出的 Unicode 文本）：按 BOM 解码", () => {
    const bytes = utf16leBytes("名称,数量\n苹果,3");
    const sheet = parseXlsx(bytes, "csv").sheets[0];
    expect(sheet.rows[1]).toEqual(["苹果", "3"]);
  });

  it("xls 扩展名但文件头不匹配：抛出解析错误", () => {
    expect(() => parseXlsx(new Uint8Array([1, 2, 3, 4]), "xls")).toThrow();
  });
});
