/** .docx 富文本编辑器共用的 TipTap 扩展（DocxEditorPane 与测试共用） */

import StarterKit from "@tiptap/starter-kit";
import {
  Table,
  TableCell,
  TableHeader,
  TableRow,
} from "@tiptap/extension-table";
import Image from "@tiptap/extension-image";

export function docxEditorExtensions() {
  return [
    StarterKit,
    Table,
    TableRow,
    TableCell,
    TableHeader,
    Image.configure({ allowBase64: true }),
  ];
}
