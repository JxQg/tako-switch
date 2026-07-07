fn main() {
    println!("cargo:rerun-if-changed=config/providers.json");
    tauri_build::build()
}
