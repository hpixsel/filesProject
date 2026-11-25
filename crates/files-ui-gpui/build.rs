fn main() {
    // The .exe and window icon (assets/app.ico, drawn by assets/make_icon.py).
    embed_resource::compile("assets/app.rc", embed_resource::NONE)
        .manifest_optional()
        .unwrap();

    // Material Icon Theme's SVGs (tools/material_icons.py) as a list of
    // ("material/<name>.svg", bytes) for the asset source.
    let dir = std::path::Path::new(&std::env::var("CARGO_MANIFEST_DIR").unwrap())
        .join("assets")
        .join("material");
    println!("cargo:rerun-if-changed={}", dir.display());
    let mut names: Vec<String> = std::fs::read_dir(&dir)
        .unwrap()
        .filter_map(|e| e.ok()?.file_name().into_string().ok())
        .filter(|n| n.ends_with(".svg"))
        .collect();
    names.sort();
    let mut code = String::from("&[\n");
    for n in names {
        let path = dir.join(&n);
        code += &format!("(\"material/{n}\", include_bytes!({path:?}) as &[u8]),\n");
    }
    code += "]\n";
    let out = std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("material.rs");
    std::fs::write(out, code).unwrap();
}
