fn main() {
    // 身份小包已声明为安装资源，缺失时 tauri_build 会直接失败；这里给出可执行的修复提示。
    #[cfg(windows)]
    {
        let msix = std::path::Path::new("target/identity/LoreKeeper.msix");
        if !msix.exists() {
            println!(
                "cargo:warning=缺少身份小包 target/identity/LoreKeeper.msix；请先运行 node scripts/build-windows-identity.mjs"
            );
        }
    }
    tauri_build::build()
}
