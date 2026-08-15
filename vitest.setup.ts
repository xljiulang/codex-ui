// 测试环境默认模拟 Tauri 运行时：ipc 层走 invoke/listen（与既有 mock 一致）。
// 远程 Web 模式（ipc.spec 等）通过删除该标记切换。
(window as unknown as { __TAURI_INTERNALS__?: unknown }).__TAURI_INTERNALS__ = {};
