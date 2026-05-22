fn main() {
    // Copy FFmpeg DLLs BEFORE tauri_build::build() so it can validate
    // the "resources/ffmpeg-dlls/*" glob in tauri.conf.json.
    #[cfg(target_os = "windows")]
    copy_ffmpeg_dlls_windows();

    tauri_build::build();
}

#[cfg(target_os = "windows")]
fn copy_ffmpeg_dlls_windows() {
    let ffmpeg_dir = match std::env::var("FFMPEG_DIR") {
        Ok(d) => std::path::PathBuf::from(d),
        Err(_) => return, // FFMPEG_DIR not set; cargo will error later when linking
    };

    println!("cargo:rerun-if-env-changed=FFMPEG_DIR");

    let bin_dir = ffmpeg_dir.join("bin");
    let manifest = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let dest_dir = manifest.join("resources").join("ffmpeg-dlls");

    if let Err(e) = std::fs::create_dir_all(&dest_dir) {
        println!("cargo:warning=Impossible de créer {}: {e}", dest_dir.display());
        return;
    }

    let entries = match std::fs::read_dir(&bin_dir) {
        Ok(e) => e,
        Err(e) => {
            println!("cargo:warning=Impossible de lire {}: {e}", bin_dir.display());
            return;
        }
    };

    for entry in entries.flatten() {
        let src = entry.path();
        if src.extension().and_then(|s| s.to_str()) != Some("dll") {
            continue;
        }
        let dst = dest_dir.join(entry.file_name());
        if let Err(e) = std::fs::copy(&src, &dst) {
            println!("cargo:warning=Copie {:?} échouée: {e}", src.file_name().unwrap_or_default());
        } else {
            println!("cargo:rerun-if-changed={}", src.display());
        }
    }
}
