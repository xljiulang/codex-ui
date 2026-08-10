fn main() {
    // tauri-build 默认不跟踪 icons/ 目录，图标变更后不会重新生成 Windows 资源；
    // 这里显式声明，图标一改就强制重编（否则 exe 一直嵌入旧图标）。
    println!("cargo:rerun-if-changed=icons/icon.ico");
    println!("cargo:rerun-if-changed=icons/32x32.png");
    println!("cargo:rerun-if-changed=icons/64x64.png");
    println!("cargo:rerun-if-changed=icons/128x128.png");
    println!("cargo:rerun-if-changed=icons/128x128@2x.png");
    println!("cargo:rerun-if-changed=icons/icon.png");
    tauri_build::build()
}
