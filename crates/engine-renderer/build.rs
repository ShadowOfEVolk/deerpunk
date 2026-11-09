use std::fmt::Write as _;
use std::path::Path;

fn embed_proggy_clean_inc(ttf_path: &Path, out_path: &Path) {
    let bytes = std::fs::read(ttf_path).unwrap_or_else(|e| {
        panic!("failed to read {}: {e}", ttf_path.display());
    });
    let mut s =
        String::from("/* Auto-generated from assets/internal/fonts/ProggyClean.ttf (MIT). */\n");
    s.push_str("static const unsigned char host_ui_proggy_clean_ttf[] = {\n");
    for (i, b) in bytes.iter().enumerate() {
        if i % 16 == 0 {
            s.push_str("    ");
        }
        let _ = write!(s, "0x{b:02x}, ");
        if i % 16 == 15 {
            s.push('\n');
        }
    }
    if !bytes.len().is_multiple_of(16) {
        s.push('\n');
    }
    let _ = write!(
        s,
        "}};\nstatic const unsigned int host_ui_proggy_clean_ttf_size = {};\n",
        bytes.len()
    );
    std::fs::write(out_path, s).unwrap_or_else(|e| {
        panic!("failed to write {}: {e}", out_path.display());
    });
}

fn main() {
    let ttf = Path::new("assets/internal/fonts/ProggyClean.ttf");
    let inc = Path::new("src/host_ui_proggyclean.inc");
    embed_proggy_clean_inc(ttf, inc);

    cc::Build::new()
        .file("src/host_ui_bind.c")
        .include("third_party")
        .include("src")
        .define("_CRT_SECURE_NO_WARNINGS", None)
        .warnings(false)
        .compile("host_ui");
    println!("cargo:rerun-if-changed=src/host_ui_bind.c");
    println!("cargo:rerun-if-changed=third_party/nuklear.h");
    println!("cargo:rerun-if-changed=assets/internal/fonts/ProggyClean.ttf");
}
