fn main() {
    // The .exe and window icon (assets/app.ico, drawn by assets/make_icon.py).
    embed_resource::compile("assets/app.rc", embed_resource::NONE)
        .manifest_optional()
        .unwrap();
}
