//! `codexui-kb` 可执行入口：参数解析与 NDJSON 协议都在 [`codexui_kb::cli`] 里。

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    std::process::exit(codexui_kb::run(args));
}
