fn main() {
    bundle_ffmpeg();
    tauri_build::build();
}

fn bundle_ffmpeg() {
    println!("cargo:rerun-if-changed=../scripts/bundle-ffmpeg.py");
    println!("cargo:rerun-if-changed=resources/ffbin/.stamp");
    let script = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("scripts")
        .join("bundle-ffmpeg.py");
    let status = std::process::Command::new("python3").arg(&script).status();
    match status {
        Ok(code) if code.success() => {}
        _ => {
            if std::env::var("PROFILE").as_deref() == Ok("release") {
                panic!(
                    "Failed to bundle ffmpeg. Install it with Homebrew (`brew install ffmpeg`)."
                );
            }
            println!("cargo:warning=ffmpeg was not bundled; relying on PATH");
        }
    }
}
