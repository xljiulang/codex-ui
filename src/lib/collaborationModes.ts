export interface CollaborationMode {
  id: "default" | "plan";
  label: string;
  desc: string;
  /** 24x24 SVG path（fill 风格），菜单与输入框 chip 共用 */
  icon: string;
}

export const COLLABORATION_MODES: CollaborationMode[] = [
  {
    id: "default",
    label: "默认模式",
    desc: "直接执行任务并给出结果",
    icon: "M8 5v14l11-7z", // 播放/运行（默认模式）
  },
  {
    id: "plan",
    label: "计划模式",
    desc: "先制定计划再执行",
    icon: "M3 5h2v2H3V5zm0 6h2v2H3v-2zm0 6h2v2H3v-2zM8 5h13v2H8V5zm0 6h13v2H8v-2zm0 6h13v2H8v-2z", // 清单/计划
  },
];

export function collaborationMode(id: string): CollaborationMode {
  return COLLABORATION_MODES.find((m) => m.id === id) ?? COLLABORATION_MODES[0];
}
