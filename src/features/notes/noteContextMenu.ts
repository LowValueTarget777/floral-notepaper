import type { TFunction } from "i18next";

const fallbackTranslate = ((key: string, options?: { defaultValue?: string }) =>
  String(options?.defaultValue ?? key)) as TFunction;

export type NoteContextMenuAction = "export" | "move" | "delete";

export interface NoteContextMenuItem {
  action: NoteContextMenuAction;
  label: string;
  tone?: "danger";
}

export function getNoteContextMenuItems(
  translate: TFunction = fallbackTranslate,
): NoteContextMenuItem[] {
  return [
    {
      action: "export",
      label: translate("noteMenu.export", { defaultValue: "导出 Markdown" }),
    },
    {
      action: "move",
      label: translate("noteMenu.moveToCategory", { defaultValue: "移动到分类…" }),
    },
    {
      action: "delete",
      label: translate("noteMenu.delete", { defaultValue: "删除笔记" }),
      tone: "danger",
    },
  ];
}

export const noteContextMenuItems: NoteContextMenuItem[] = [
  { action: "export", label: "导出 Markdown" },
  { action: "move", label: "移动到分类…" },
  { action: "delete", label: "删除笔记", tone: "danger" },
];
