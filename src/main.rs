use sfbinpack::{CompressedTrainingDataEntryReader, CompressedTrainingDataEntryWriter, TrainingDataEntry};
use sfbinpack::chess::piece::Piece;
use std::env;
use std::fs::{self, File};
use std::io::{self, Write};
use std::panic;
use std::path::Path;
use std::time::Instant;

fn print_usage() {
    println!("📦 Usage: binpack_processor --input <dir> --output <file>");
    println!("\n⚙️  Options:");
    println!("  -i, --input <dir>    Directory containing the source .binpack files");
    println!("  -o, --output <file>  Path to save the final merged .binpack file");
}

fn render_progress_bar(completed: usize, total: usize, start_time: Instant) {
    let width = 30;
    let percentage = if total > 0 { (completed as f64 / total as f64) * 100.0 } else { 0.0 };
    let filled = if total > 0 { (completed * width) / total } else { 0 };
    
    let mut bar = String::new();
    for i in 0..width {
        if i < filled {
            bar.push('█');
        } else {
            bar.push('░');
        }
    }

    let emoji = match percentage {
        p if p >= 100.0 => "🔥",
        p if p >= 50.0  => "⚡",
        _               => "⏳",
    };

    let elapsed = start_time.elapsed().as_secs();

    print!(
        "\r{} [{}] {:.1}% ({}/{} files) | ⏱️  {}s",
        emoji, bar, percentage, completed, total, elapsed
    );
    let _ = io::stdout().flush();
}

fn is_valid_entry(entry: &TrainingDataEntry) -> bool {
    if entry.score.abs() > 32000 {
        return false;
    }

    if entry.pos.fen().is_err() {
        return false;
    }

    let from = entry.mv.from();
    let to = entry.mv.to();
    if from.index() >= 64 || to.index() >= 64 {
        return false;
    }

    if entry.pos.piece_at(from) == Piece::none() {
        return false;
    }

    true
}

fn sequential_merge_and_validate(
    input_dir: &str,
    output_path: &str,
) -> Result<(), Box<dyn std::error::Error>> {
    let start_time = Instant::now();

    // 1. Scan the input directory for .binpack files
    let mut input_paths = Vec::new();
    for entry in fs::read_dir(input_dir)? {
        let entry = entry?;
        let path = entry.path();
        if path.is_file() && path.extension().map_or(false, |ext| ext == "binpack") {
            if let Some(path_str) = path.to_str() {
                input_paths.push(path_str.to_string());
            }
        }
    }

    let total_files = input_paths.len();
    if total_files == 0 {
        println!("❌ No .binpack files found in directory: {}", input_dir);
        return Ok(());
    }

    println!("🔍 Found {} source files to process.", total_files);

    // 2. Initialize the single-threaded master file writer stream
    let out_file = File::create(output_path)?;
    let mut writer = CompressedTrainingDataEntryWriter::new(out_file).unwrap();
    let mut total_positions_written = 0;
    let mut files_completed = 0;

    render_progress_bar(0, total_files, start_time);

    // 3. Process files sequentially to maintain move-by-move delta stream integrity
    for path in input_paths {
        let in_file = match File::open(&path) {
            Ok(f) => f,
            Err(_) => {
                files_completed += 1;
                render_progress_bar(files_completed, total_files, start_time);
                continue;
            }
        };

        let mut reader = match CompressedTrainingDataEntryReader::new(in_file) {
            Ok(r) => r,
            Err(_) => {
                eprintln!("\n⚠️  Skipping file due to header corruption: {}", path);
                files_completed += 1;
                render_progress_bar(files_completed, total_files, start_time);
                continue;
            }
        };

        // Standard iteration block matching sfbinpack design properties
        while reader.has_next() {
            let entry = match panic::catch_unwind(panic::AssertUnwindSafe(|| reader.next())) {
                Ok(entry) => entry,
                Err(_) => {
                    eprintln!("\n⚠️  Corrupted move stream detected in file {}, skipping remaining entries.", path);
                    break;
                }
            };

            if !is_valid_entry(&entry) {
                continue;
            }

            let write_result = panic::catch_unwind(panic::AssertUnwindSafe(|| writer.write_entry(&entry)));
            match write_result {
                Ok(Ok(())) => {
                    total_positions_written += 1;
                }
                Ok(Err(e)) => {
                    eprintln!("\n⚠️  Skipping invalid entry in file {}: {}", path, e);
                    continue;
                }
                Err(_) => {
                    eprintln!("\n⚠️  Panic while writing entry from file {}, skipping remaining entries.", path);
                    break;
                }
            }
        }

        files_completed += 1;
        render_progress_bar(files_completed, total_files, start_time);
    }

    // Flush and seal the binpack structural frames to disk on closure
    writer.flush_and_end();

    println!(
        "\n\n🎉 Success! Processed and merged {} positions across files.",
        total_positions_written
    );
    println!("💾 Output file generated cleanly: {}", output_path);

    Ok(())
}

fn main() {
    let args: Vec<String> = env::args().collect();
    
    let mut input_dir = String::new();
    let mut output_path = String::new();
    
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "-i" | "--input" => {
                if i + 1 < args.len() {
                    input_dir = args[i + 1].clone();
                    i += 2;
                } else {
                    eprintln!("❌ Error: Missing value for --input");
                    print_usage();
                    std::process::exit(1);
                }
            }
            "-o" | "--output" => {
                if i + 1 < args.len() {
                    output_path = args[i + 1].clone();
                    i += 2;
                } else {
                    eprintln!("❌ Error: Missing value for --output");
                    print_usage();
                    std::process::exit(1);
                }
            }
            _ => {
                eprintln!("❌ Error: Unknown argument '{}'", args[i]);
                print_usage();
                std::process::exit(1);
            }
        }
    }

    if input_dir.is_empty() || output_path.is_empty() {
        eprintln!("❌ Error: Missing required operational parameters.");
        print_usage();
        std::process::exit(1);
    }

    if !Path::new(&input_dir).exists() {
        eprintln!("❌ Error: Target input directory '{}' can't be resolved.", input_dir);
        std::process::exit(1);
    }

    panic::set_hook(Box::new(|_| {}));

    if let Err(e) = sequential_merge_and_validate(&input_dir, &output_path) {
        eprintln!("\n❌ Severe process break: {}", e);
        std::process::exit(1);
    }
}
