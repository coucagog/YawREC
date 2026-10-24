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

    let entries: Vec<_> = match std::fs::read_dir(&bin_dir) {
        Ok(e) => e.flatten().collect(),
        Err(e) => {
            println!("cargo:warning=Impossible de lire {}: {e}", bin_dir.display());
            return;
        }
    };

    // DLL d'une autre version de FFmpeg laissées par un build précédent
    // (ex. avcodec-63.dll après un passage par FFmpeg 9) : elles seraient
    // embarquées dans l'installeur et brouillent le diagnostic quand on
    // teste l'exe directement. On les retire du dossier de ressources et
    // du dossier de l'exe (target/<profil>, où Tauri recopie les ressources).
    let current: std::collections::HashSet<std::ffi::OsString> =
        entries.iter().map(|e| e.file_name()).collect();
    let profile_dir = std::env::var("OUT_DIR")
        .ok()
        .and_then(|o| std::path::PathBuf::from(o).ancestors().nth(3).map(|p| p.to_path_buf()));
    for dir in std::iter::once(dest_dir.clone()).chain(profile_dir) {
        remove_stale_ffmpeg_dlls(&dir, &current);
    }

    for entry in entries {
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

/// Supprime de `dir` les DLL FFmpeg (avcodec-61.dll, swscale-8.dll…) absentes
/// de `current`, c'est-à-dire provenant d'une autre version de FFmpeg.
#[cfg(target_os = "windows")]
fn remove_stale_ffmpeg_dlls(
    dir: &std::path::Path,
    current: &std::collections::HashSet<std::ffi::OsString>,
) {
    const FFMPEG_LIBS: [&str; 8] = [
        "avcodec-", "avdevice-", "avfilter-", "avformat-",
        "avutil-", "postproc-", "swresample-", "swscale-",
    ];
    let Ok(entries) = std::fs::read_dir(dir) else { return };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let lower = name.to_string_lossy().to_ascii_lowercase();
        let is_ffmpeg = lower.ends_with(".dll") && FFMPEG_LIBS.iter().any(|p| lower.starts_with(p));
        if is_ffmpeg && !current.contains(&name) {
            match std::fs::remove_file(entry.path()) {
                Ok(()) => println!("cargo:warning=DLL FFmpeg obsolète retirée : {}", entry.path().display()),
                Err(e) => println!("cargo:warning=Impossible de retirer {}: {e}", entry.path().display()),
            }
        }
    }
}
