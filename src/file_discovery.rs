use anyhow::Result;
use itertools::Itertools;
use lexical_sort::natural_lexical_cmp;
use std::path::{Path, PathBuf};
use walkdir::WalkDir;

/// Discovers input files from the given path.
/// If the path is a file, returns it as a single-item vector.
/// If the path is a directory, recursively finds all .vpy files, excluding already processed ones.
pub fn discover_input_files(input_path: &Path) -> Result<Vec<PathBuf>> {
    if input_path.is_file() {
        if is_vpy_file(input_path) {
            Ok(vec![input_path.to_path_buf()])
        } else {
            anyhow::bail!(
                "Input file must be a .vpy script: {}",
                input_path.to_string_lossy()
            )
        }
    } else if input_path.is_dir() {
        let files = WalkDir::new(input_path)
            .into_iter()
            .filter_map(|e| e.ok())
            .filter(|e| is_vpy_file(e.path()))
            .filter(|e| !is_processed_file(e.path()))
            .map(|e| e.path().to_path_buf())
            .sorted_unstable_by(|a, b| {
                natural_lexical_cmp(&a.to_string_lossy(), &b.to_string_lossy())
            })
            .collect();
        Ok(files)
    } else {
        anyhow::bail!("Input path is neither a file nor a directory")
    }
}

/// Checks if a file is a VapourSynth Python script (.vpy file)
fn is_vpy_file(path: &Path) -> bool {
    path.extension()
        .map(|ext| ext.to_string_lossy().to_string())
        == Some("vpy".to_string())
}

/// Checks if a file appears to be already processed based on its filename patterns
fn is_processed_file(path: &Path) -> bool {
    let filestem = path
        .file_stem()
        .expect("File should have a name")
        .to_string_lossy();

    filestem.contains(".aom-q")
        || filestem.contains(".rav1e-q")
        || filestem.contains(".svt-q")
        || filestem.contains(".x264-q")
        || filestem.contains(".x265-q")
        || filestem.ends_with(".copy")
}
