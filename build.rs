fn main() {
    #[cfg(target_os = "windows")]
    {
        embed_resource::compile("assets/notepad-studio.rc", embed_resource::NONE)
            .manifest_required()
            .unwrap();
        println!("cargo:rerun-if-changed=assets/notepad-studio.rc");
        println!("cargo:rerun-if-changed=assets/icon.ico");
    }
}
