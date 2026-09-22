use std::path::{Path, PathBuf};

/// 单次编译的中间产物与最终输出文件路径。
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

/// 获取平台特定的可执行文件扩展名
fn get_exe_ext() -> &'static str {
    if cfg!(target_os = "windows") {
        "exe"
    } else {
        ""
    }
}

/// 获取平台特定的目标文件扩展名
fn get_obj_ext() -> &'static str {
    if cfg!(target_os = "windows") {
        "obj"
    } else {
        "o"
    }
}

/// 构建带平台特定扩展名的输出路径
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

/// 获取中间产物文件路径（与输出位于同一目录）
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
