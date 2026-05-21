//! `freemkv-tools labels-extract` — dump the entire `/BDMV/` tree from
//! a captured disc image to a directory on disk, preserving the disc's
//! subdirectory structure.
//!
//! Companion to `labels-analyze`: where analyze reports what the
//! parsers SAW, extract gives you the raw bytes so you can grep,
//! `strings`, `xxd`, or open them in an editor. Use it to investigate
//! null-parser discs (what authoring framework are they?) or pixelogic
//! discs that emit suspiciously few labels (what tokens are in the
//! .bin we don't recognize?).
//!
//! Args:
//!   <image>     disc image file (1 GB head capture or full ISO)
//!   <out-dir>   directory to extract into; created if missing

use libfreemkv::FileSectorSource;
use libfreemkv::read_filesystem;
use std::path::{Path, PathBuf};

const SKIP_LARGER_THAN: u64 = 64 * 1024 * 1024;

pub fn run(argv: &[String]) -> Result<(), String> {
    if argv.len() < 2 || argv[0] == "--help" || argv[0] == "-h" {
        println!("{}", help_text());
        return Ok(());
    }
    let image_path_str = &argv[0];
    let out_dir = PathBuf::from(&argv[1]);
    let image_path = Path::new(image_path_str);

    let mut reader = FileSectorSource::open(image_path)
        .map_err(|e| format!("open {}: {}", image_path_str, e))?;
    let udf = read_filesystem(&mut reader).map_err(|e| format!("udf read_filesystem: {:?}", e))?;

    std::fs::create_dir_all(&out_dir)
        .map_err(|e| format!("create {}: {}", out_dir.display(), e))?;

    // BFS over /BDMV using a worklist of (udf_path, fs_path). We only
    // ever name the path strings, never `DirEntry` directly — the type
    // isn't re-exported from libfreemkv at the crate root.
    let mut work: Vec<(String, PathBuf)> = vec![("/BDMV".to_string(), out_dir.join("BDMV"))];
    let mut written = 0usize;
    let mut skipped_size = 0usize;
    let mut skipped_read = 0usize;

    while let Some((udf_path, fs_path)) = work.pop() {
        let dir = match udf.find_dir(&udf_path) {
            Some(d) => d,
            None => continue,
        };
        std::fs::create_dir_all(&fs_path)
            .map_err(|e| format!("mkdir {}: {}", fs_path.display(), e))?;
        for entry in &dir.entries {
            let next_udf = format!("{}/{}", udf_path, entry.name);
            let next_fs = fs_path.join(&entry.name);
            if entry.is_dir {
                work.push((next_udf, next_fs));
            } else {
                if entry.size > SKIP_LARGER_THAN {
                    // Encrypted m2ts payloads, etc — useless for labels.
                    skipped_size += 1;
                    continue;
                }
                match udf.read_file(&mut reader, &next_udf) {
                    Ok(bytes) => {
                        std::fs::write(&next_fs, &bytes)
                            .map_err(|e| format!("write {}: {}", next_fs.display(), e))?;
                        written += 1;
                    }
                    Err(_) => {
                        // File's sectors are past the 1 GB head
                        // capture — skip silently and count.
                        skipped_read += 1;
                    }
                }
            }
        }
    }

    eprintln!(
        "extracted {} files, skipped {} large (m2ts), skipped {} read-errors (past capture window)",
        written, skipped_size, skipped_read
    );
    Ok(())
}

fn help_text() -> &'static str {
    "freemkv-tools labels-extract — dump /BDMV/ tree from a disc image

Usage: freemkv-tools labels-extract <image> <out-dir>

  <image>    disc image file (1 GB head capture or full ISO)
  <out-dir>  destination dir; created if missing; tree mirrors disc

Files >64 MB are skipped (m2ts payloads). Files whose sectors fall
past the 1 GB capture window are also skipped silently — counts are
reported on stderr."
}
