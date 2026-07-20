fn main() {
    println!("cargo:rerun-if-changed=icons/app.ico");
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var_os("CARGO_CFG_WINDOWS").is_some() {
        let mut res = winresource::WindowsResource::new();
        res.set_icon("icons/app.ico");
        // Shown in the exe's Properties > Details tab.
        res.set("ProductName", "AutoMouse");
        res.set("FileDescription", "AutoMouse");
        res.set("CompanyName", "Adrien Boitelle");
        res.set("LegalCopyright", "Copyright (C) 2026 Adrien Boitelle, GPLv3");
        res.compile().expect("failed to embed icon resource");
    }
}
