fn main() {
    println!("cargo:rerun-if-changed=config/providers.default.json");
    tauri_build::build()
}
