import { listen, type UnlistenFn } from "@tauri-apps/api/event";

interface TerminalOutputPayload {
  id: string;
  data: string;
}

interface TerminalExitPayload {
  id: string;
  exitCode: number;
}

/**
 * 终端会话事件桥：在 terminal_spawn 之前注册全局监听并按 id 缓冲输出，
 * 解决懒加载 TerminalPane 挂载前启动输出（含 ConPTY DSR 查询）丢失的问题。
 */
export interface TerminalHandle {
  /** 取走并清空挂载前缓冲的全部输出（含 DSR 查询，xterm 写入后自动应答） */
  flush(): string[];
  /** 挂载前已发生的退出码；无则为 null */
  readonly exitCode: number | null;
  onData(cb: (data: string) => void): void;
  onExit(cb: (exitCode: number) => void): void;
  detach(): void;
}

/** 无活跃订阅者时的输出缓冲：id -> 输出块列表（启动期少量数据） */
const outputBuffer = new Map<string, string[]>();
/** 无活跃订阅者时的退出码：id -> 退出码 */
const exitBuffer = new Map<string, number>();
const dataHandlers = new Map<string, Set<(data: string) => void>>();
const exitHandlers = new Map<string, Set<(exitCode: number) => void>>();

let unlistenOutput: UnlistenFn | null = null;
let unlistenExit: UnlistenFn | null = null;
let listenersReady: Promise<void> | null = null;

/**
 * 注册 terminal/output 与 terminal/exit 全局监听（幂等，应用生命周期内常驻，
 * 模式对齐 session-fs/changed）。必须在 terminal_spawn 之前完成，避免丢事件。
 */
export function ensureTerminalListeners(): Promise<void> {
  if (listenersReady) return listenersReady;
  listenersReady = (async () => {
    unlistenOutput = await listen<TerminalOutputPayload>(
      "terminal/output",
      (e) => {
        const { id, data } = e.payload;
        const handlers = dataHandlers.get(id);
        if (handlers && handlers.size > 0) {
          for (const cb of handlers) cb(data);
        } else {
          const buf = outputBuffer.get(id);
          if (buf) buf.push(data);
          else outputBuffer.set(id, [data]);
        }
      },
    );
    unlistenExit = await listen<TerminalExitPayload>("terminal/exit", (e) => {
      const { id, exitCode } = e.payload;
      const handlers = exitHandlers.get(id);
      if (handlers && handlers.size > 0) {
        for (const cb of handlers) cb(exitCode);
      } else {
        exitBuffer.set(id, exitCode);
      }
    });
  })();
  return listenersReady;
}

/**
 * 建立（或复用）某个终端 id 的订阅：调用方先 flush 回放缓冲，再注册
 * onData/onExit 接收实时事件；卸载时 detach。可被 openTerminalTab 与
 * TerminalPane 先后调用，状态共享。
 */
export function attachTerminal(id: string): TerminalHandle {
  if (!outputBuffer.has(id)) outputBuffer.set(id, []);
  if (!dataHandlers.has(id)) dataHandlers.set(id, new Set());
  if (!exitHandlers.has(id)) exitHandlers.set(id, new Set());
  let detached = false;
  return {
    flush(): string[] {
      const buf = outputBuffer.get(id) ?? [];
      outputBuffer.delete(id);
      return buf;
    },
    get exitCode(): number | null {
      return exitBuffer.get(id) ?? null;
    },
    onData(cb) {
      dataHandlers.get(id)?.add(cb);
    },
    onExit(cb) {
      exitHandlers.get(id)?.add(cb);
    },
    detach() {
      if (detached) return;
      detached = true;
      dataHandlers.get(id)?.clear();
      exitHandlers.get(id)?.clear();
    },
  };
}

/** 终端标签关闭时释放该 id 的缓冲与订阅状态，防止泄漏 */
export function releaseTerminal(id: string): void {
  outputBuffer.delete(id);
  exitBuffer.delete(id);
  dataHandlers.delete(id);
  exitHandlers.delete(id);
}

/** 测试专用：重置监听、缓冲与订阅状态 */
export function __resetTerminalEventsForTest(): void {
  unlistenOutput?.();
  unlistenExit?.();
  unlistenOutput = null;
  unlistenExit = null;
  listenersReady = null;
  outputBuffer.clear();
  exitBuffer.clear();
  dataHandlers.clear();
  exitHandlers.clear();
}
