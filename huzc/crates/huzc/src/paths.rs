use std::path::{Path, PathBuf};

/// Intermediate and output file paths for one compilation.
pub struct OutputPaths {
    pub exe_path: PathBuf,
    pub ll_path: PathBuf,
    pub obj_path: PathBuf,
}

impl OutputPaths {
    pub fn new(output: &str) -> Self {
        Self {
            exe_path: build_output_path(output),
            ll_path: build_intermediate_path(output, "ll"),
            obj_path: build_intermediate_path(output, get_obj_ext()),
        }
    }
}

/// Get platform-specific executable extension
fn get_exe_ext() -> &'static str {
    if cfg!(target_os = "windows") {
        "exe"
    } else {
        ""
    }
}

/// Get platform-specific object file extension
fn get_obj_ext() -> &'static str {
    if cfg!(target_os = "windows") {
        "obj"
    } else {
        "o"
    }
}

/// Build output path with platform-specific extension
fn build_output_path(output: &str) -> PathBuf {
    if output.ends_with(".exe") || output.ends_with(".o") || output.ends_with(".obj") {
        PathBuf::from(output)
    } else {
        let ext = get_exe_ext();
        if ext.is_empty() {
            PathBuf::from(output)
        } else {
            PathBuf::from(format!("{}.{}", output, ext))
        }
    }
}

/// Get intermediate file path (same directory as output)
fn build_intermediate_path(output: &str, ext: &str) -> PathBuf {
    let output_path = PathBuf::from(output);
    let output_dir = output_path.parent().unwrap_or(Path::new(""));
    let stem = output_path.file_stem().unwrap().to_str().unwrap();

    if output_dir.as_os_str().is_empty() {
        PathBuf::from(format!("{}.{}", stem, ext))
    } else {
        output_dir.join(format!("{}.{}", stem, ext))
    }
}
