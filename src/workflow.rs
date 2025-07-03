use anyhow::Result;
use colored::*;
use std::path::Path;
use std::sync::{Arc, atomic::AtomicBool};

use crate::file_discovery::discover_input_files;
use crate::output_configuration::parse_output_configurations;
use crate::process_file;

/// Main workflow that processes all input files with their configurations
#[allow(clippy::too_many_arguments)]
pub fn run_processing_workflow(
    input_path: &Path,
    formats: Option<&str>,
    output_dir: Option<&Path>,
    keep_lossless: bool,
    lossless_only: bool,
    skip_lossless: bool,
    copy_audio_to_lossless: bool,
    force_keyframes: Option<&str>,
    verify_frame_count: bool,
    ignore_delay: bool,
    no_retry: bool,
    sigterm: Arc<AtomicBool>,
) -> Result<()> {
    let inputs = discover_input_files(input_path)?;

    if inputs.is_empty() {
        eprintln!("{} No input files found", "[Warning]".yellow().bold());
        return Ok(());
    }

    eprintln!(
        "{} Found {} input file(s)",
        "[Info]".blue().bold(),
        inputs.len().to_string().blue().bold()
    );

    for input in inputs {
        if sigterm.load(std::sync::atomic::Ordering::Relaxed) {
            eprintln!(
                "{} Processing interrupted by signal",
                "[Info]".blue().bold()
            );
            break;
        }

        let outputs = parse_output_configurations(formats, &input);

        let result = process_file(
            &input,
            &outputs,
            output_dir,
            keep_lossless,
            lossless_only,
            skip_lossless,
            copy_audio_to_lossless,
            force_keyframes,
            verify_frame_count,
            ignore_delay,
            no_retry,
            Arc::clone(&sigterm),
        );

        if let Err(err) = result {
            eprintln!(
                "{} Failed processing file {}: {}",
                "[Error]".red().bold(),
                input
                    .file_name()
                    .expect("File should have a name")
                    .to_string_lossy()
                    .red(),
                err.to_string().red()
            );
        }

        eprintln!();
    }

    Ok(())
}
