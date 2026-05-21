//! `freemkv-tools labels-corpus-check` — corpus regression harness for
//! the BD-J label parsers in `libfreemkv`.
//!
//! Walks a corpus directory of disc captures (`*.bin` head captures or
//! full `*.iso`) and, for each, runs the parser stack via
//! `libfreemkv::labels::analyze`. If a sibling
//! `<basename>.snapshot.json` exists, diffs the current output against
//! it and reports any drift. The first time you run it on a disc,
//! pass `--update` to bless the current output as the snapshot.
//!
//! Why this exists: every parser refactor (vocab tightening, helper
//! consolidation, new framework support) needs a fast way to prove
//! it didn't regress any disc we already handle. Without this you
//! find out at the next live rip — too late. Drive the diff into CI
//! once we have a stable corpus directory.
//!
//! Args:
//!   <corpus-dir>      directory of disc captures
//!   --update          bless current outputs as new snapshots (NO diff)
//!   --exact           strict byte-for-byte JSON diff (default: structural —
//!                     parser, label counts, per-stream fields. Ignores
//!                     raw_codes and jar_inventory which jitter across
//!                     capture sizes / UDF variations.)
//!   --filter <glob>   only check disc captures whose filename matches
//!                     the substring (e.g. --filter disc-07)
//!
//! Output: one line per disc summarizing pass/fail, then per-failure
//! detail blocks. Exits non-zero on any drift so the harness can gate
//! CI.

use libfreemkv::FileSectorSource;
use libfreemkv::labels::{StreamLabelType, analyze};
use libfreemkv::read_filesystem;
use serde_json::{Value, json};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};

pub fn run(argv: &[String]) -> Result<(), String> {
    let opts = Options::parse(argv)?;
    if opts.help {
        println!("{}", help_text());
        return Ok(());
    }

    let captures = list_captures(&opts.corpus_dir, opts.filter.as_deref())?;
    if captures.is_empty() {
        return Err(format!(
            "no disc captures (*.bin or *.iso) found under {}",
            opts.corpus_dir.display()
        ));
    }

    let mut pass = 0usize;
    let mut fail = 0usize;
    let mut no_snapshot = 0usize;
    let mut details: Vec<String> = Vec::new();

    for capture in &captures {
        let basename = capture.file_stem().unwrap_or_default().to_string_lossy();
        let snapshot_path = capture.with_extension("snapshot.json");

        let current = match build_analysis_json(capture) {
            Ok(v) => v,
            Err(e) => {
                fail += 1;
                println!("FAIL {:<24} (analyze error: {})", basename, e);
                continue;
            }
        };

        if opts.update {
            let pretty = serde_json::to_string_pretty(&current)
                .map_err(|e| format!("serialize {}: {}", basename, e))?;
            fs::write(&snapshot_path, pretty)
                .map_err(|e| format!("write {}: {}", snapshot_path.display(), e))?;
            println!("UPDATE {:<22} -> {}", basename, snapshot_path.display());
            continue;
        }

        let snapshot = match fs::read_to_string(&snapshot_path) {
            Ok(t) => match serde_json::from_str::<Value>(&t) {
                Ok(v) => v,
                Err(e) => {
                    fail += 1;
                    println!(
                        "FAIL {:<24} (snapshot parse error in {}: {})",
                        basename,
                        snapshot_path.display(),
                        e
                    );
                    continue;
                }
            },
            Err(_) => {
                no_snapshot += 1;
                println!(
                    "MISS {:<24} (no {}; run with --update to create)",
                    basename,
                    snapshot_path
                        .file_name()
                        .unwrap_or_default()
                        .to_string_lossy()
                );
                continue;
            }
        };

        let diffs = if opts.exact {
            diff_exact(&snapshot, &current)
        } else {
            diff_structural(&snapshot, &current)
        };

        if diffs.is_empty() {
            pass += 1;
            println!("PASS {}", basename);
        } else {
            fail += 1;
            println!(
                "FAIL {:<24} ({} diff{})",
                basename,
                diffs.len(),
                if diffs.len() == 1 { "" } else { "s" }
            );
            let mut block = format!("  --- {} ---\n", basename);
            for d in &diffs {
                block.push_str("  ");
                block.push_str(d);
                block.push('\n');
            }
            details.push(block);
        }
    }

    println!();
    println!(
        "Summary: {} pass, {} fail, {} missing snapshot ({} total)",
        pass,
        fail,
        no_snapshot,
        captures.len()
    );
    if !details.is_empty() {
        println!();
        for block in details {
            print!("{}", block);
        }
    }

    if fail > 0 {
        return Err(format!(
            "{} regression{}",
            fail,
            if fail == 1 { "" } else { "s" }
        ));
    }
    Ok(())
}

struct Options {
    corpus_dir: PathBuf,
    update: bool,
    exact: bool,
    filter: Option<String>,
    help: bool,
}

impl Options {
    fn parse(argv: &[String]) -> Result<Self, String> {
        let mut corpus_dir: Option<PathBuf> = None;
        let mut update = false;
        let mut exact = false;
        let mut filter: Option<String> = None;
        let mut help = false;
        let mut i = 0;
        while i < argv.len() {
            match argv[i].as_str() {
                "--help" | "-h" => help = true,
                "--update" => update = true,
                "--exact" => exact = true,
                "--filter" => {
                    i += 1;
                    if i >= argv.len() {
                        return Err("--filter requires an argument".into());
                    }
                    filter = Some(argv[i].clone());
                }
                other => {
                    if corpus_dir.is_some() {
                        return Err(format!("unexpected positional argument: {}", other));
                    }
                    corpus_dir = Some(PathBuf::from(other));
                }
            }
            i += 1;
        }
        Ok(Options {
            corpus_dir: corpus_dir.unwrap_or_else(|| PathBuf::from(".")),
            update,
            exact,
            filter,
            help,
        })
    }
}

fn list_captures(dir: &Path, filter: Option<&str>) -> Result<Vec<PathBuf>, String> {
    if !dir.is_dir() {
        return Err(format!("not a directory: {}", dir.display()));
    }
    let mut out = Vec::new();
    walk(dir, &mut out, filter)?;
    out.sort();
    Ok(out)
}

fn walk(dir: &Path, out: &mut Vec<PathBuf>, filter: Option<&str>) -> Result<(), String> {
    let entries = fs::read_dir(dir).map_err(|e| format!("read {}: {}", dir.display(), e))?;
    for entry in entries {
        let entry = entry.map_err(|e| format!("entry in {}: {}", dir.display(), e))?;
        let path = entry.path();
        if path.is_dir() {
            walk(&path, out, filter)?;
            continue;
        }
        let Some(ext) = path.extension().and_then(|s| s.to_str()) else {
            continue;
        };
        if ext != "bin" && ext != "iso" {
            continue;
        }
        let name = path.file_name().unwrap_or_default().to_string_lossy();
        if let Some(f) = filter {
            if !name.contains(f) {
                continue;
            }
        }
        out.push(path);
    }
    Ok(())
}

fn build_analysis_json(path: &Path) -> Result<Value, String> {
    let mut reader =
        FileSectorSource::open(path).map_err(|e| format!("open {}: {}", path.display(), e))?;
    let udf = read_filesystem(&mut reader).map_err(|e| format!("udf: {:?}", e))?;
    let result = analyze(&mut reader, &udf);

    let labels: Vec<Value> = result
        .labels
        .iter()
        .map(|l| {
            json!({
                "stream_number": l.stream_number,
                "stream_type": match l.stream_type {
                    StreamLabelType::Audio => "audio",
                    StreamLabelType::Subtitle => "subtitle",
                },
                "language": l.language,
                "name": l.name,
                "codec_hint": l.codec_hint,
                "variant": l.variant,
                "purpose": format!("{:?}", l.purpose),
                "qualifier": format!("{:?}", l.qualifier),
            })
        })
        .collect();

    Ok(json!({
        "parser": result.parser,
        "confidence": result.confidence.map(|c| format!("{:?}", c)),
        "parsers_detected": result.parsers_detected,
        "audio_count": result.labels.iter().filter(|l| l.stream_type == StreamLabelType::Audio).count(),
        "subtitle_count": result.labels.iter().filter(|l| l.stream_type == StreamLabelType::Subtitle).count(),
        "labels": labels,
        // jar_inventory and raw_codes intentionally excluded from the
        // structural diff (they shift across capture sizes); included
        // here so --exact mode and human inspection still see them.
        "jar_inventory": result.jar_inventory,
    }))
}

/// Strict byte-for-byte (well, value-for-value) JSON comparison.
fn diff_exact(want: &Value, got: &Value) -> Vec<String> {
    if want == got {
        return Vec::new();
    }
    // Produce a single diff line per top-level key that differs.
    let mut out = Vec::new();
    let want_obj = want.as_object();
    let got_obj = got.as_object();
    if let (Some(w), Some(g)) = (want_obj, got_obj) {
        let mut keys: std::collections::BTreeSet<&String> = w.keys().collect();
        keys.extend(g.keys());
        for k in keys {
            if w.get(k) != g.get(k) {
                out.push(format!(
                    "{}: want={} got={}",
                    k,
                    json_brief(w.get(k)),
                    json_brief(g.get(k))
                ));
            }
        }
    } else {
        out.push(format!(
            "root: want={} got={}",
            json_brief(Some(want)),
            json_brief(Some(got))
        ));
    }
    out
}

/// Structural diff — compares the fields that are stable across
/// capture-size and UDF-jitter changes:
///   - parser, parsers_detected
///   - audio_count, subtitle_count
///   - labels (per-stream language / purpose / qualifier / codec_hint /
///     variant / name — but order is normalized by (stream_type,
///     stream_number) so capture variance doesn't affect ordering)
///
/// Excluded: jar_inventory, raw_codes (those shift with capture size
/// and UDF table layout).
fn diff_structural(want: &Value, got: &Value) -> Vec<String> {
    let mut out = Vec::new();

    for key in [
        "parser",
        "confidence",
        "parsers_detected",
        "audio_count",
        "subtitle_count",
    ] {
        if want.get(key) != got.get(key) {
            out.push(format!(
                "{}: want={} got={}",
                key,
                json_brief(want.get(key)),
                json_brief(got.get(key))
            ));
        }
    }

    let want_labels = normalize_labels(want.get("labels"));
    let got_labels = normalize_labels(got.get("labels"));
    if want_labels.len() != got_labels.len() {
        out.push(format!(
            "labels.len: want={} got={}",
            want_labels.len(),
            got_labels.len()
        ));
    }
    for (i, (w, g)) in want_labels.iter().zip(got_labels.iter()).enumerate() {
        for key in [
            "stream_number",
            "stream_type",
            "language",
            "name",
            "codec_hint",
            "variant",
            "purpose",
            "qualifier",
        ] {
            if w.get(key) != g.get(key) {
                out.push(format!(
                    "labels[{}].{}: want={} got={}",
                    i,
                    key,
                    json_brief(w.get(key)),
                    json_brief(g.get(key))
                ));
            }
        }
    }

    out
}

fn normalize_labels(v: Option<&Value>) -> Vec<BTreeMap<String, Value>> {
    let Some(Value::Array(arr)) = v else {
        return Vec::new();
    };
    let mut out: Vec<BTreeMap<String, Value>> = arr
        .iter()
        .filter_map(|v| v.as_object())
        .map(|o| {
            o.iter()
                .map(|(k, v)| (k.clone(), v.clone()))
                .collect::<BTreeMap<_, _>>()
        })
        .collect();
    // Sort by (stream_type, stream_number) so capture-order variance
    // doesn't show up as a diff.
    out.sort_by(|a, b| {
        let ta = a.get("stream_type").and_then(|v| v.as_str()).unwrap_or("");
        let tb = b.get("stream_type").and_then(|v| v.as_str()).unwrap_or("");
        let na = a.get("stream_number").and_then(|v| v.as_u64()).unwrap_or(0);
        let nb = b.get("stream_number").and_then(|v| v.as_u64()).unwrap_or(0);
        (ta, na).cmp(&(tb, nb))
    });
    out
}

fn json_brief(v: Option<&Value>) -> String {
    match v {
        None => "<missing>".to_string(),
        Some(v) => {
            let s = v.to_string();
            if s.len() > 80 {
                format!("{}…", &s[..80])
            } else {
                s
            }
        }
    }
}

fn help_text() -> &'static str {
    "freemkv-tools labels-corpus-check — regression harness for libfreemkv label parsers

Usage: freemkv-tools labels-corpus-check <corpus-dir> [options]

Walks <corpus-dir> recursively for *.bin and *.iso disc captures.
For each, runs the parser stack and either:
  - compares against <basename>.snapshot.json (default), or
  - blesses current output as the new snapshot (--update).

Options:
  --update          write/overwrite <basename>.snapshot.json with current output.
                    No diff is performed in this mode.
  --exact           strict byte-for-byte JSON comparison.
                    Default: structural diff (parser, counts, per-stream fields;
                    ignores jar_inventory and raw_codes which jitter).
  --filter <text>   only check captures whose filename contains <text>
                    (e.g. --filter disc-07).
  --help, -h        this message.

Exit code: 0 if all PASS, non-zero on any FAIL.

Examples:
  freemkv-tools labels-corpus-check /srv/autorip/labels-corpus/
  freemkv-tools labels-corpus-check /srv/autorip/labels-corpus/ --update
  freemkv-tools labels-corpus-check /srv/autorip/labels-corpus/ --filter dune"
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn diff_structural_clean_when_identical() {
        let v = json!({
            "parser": "pixelogic",
            "parsers_detected": ["pixelogic"],
            "audio_count": 2,
            "subtitle_count": 1,
            "labels": [
                {"stream_number": 1, "stream_type": "audio", "language": "eng", "name": "",
                 "codec_hint": "TrueHD", "variant": "", "purpose": "Normal", "qualifier": "None"},
                {"stream_number": 2, "stream_type": "audio", "language": "spa", "name": "",
                 "codec_hint": "Dolby Digital", "variant": "", "purpose": "Normal", "qualifier": "None"},
                {"stream_number": 1, "stream_type": "subtitle", "language": "eng", "name": "",
                 "codec_hint": "", "variant": "", "purpose": "Normal", "qualifier": "Sdh"},
            ],
            "jar_inventory": ["bluray_project.bin", "playlists.xml"],
        });
        assert!(diff_structural(&v, &v).is_empty());
    }

    #[test]
    fn diff_structural_ignores_jar_inventory_drift() {
        let want = json!({
            "parser": "pixelogic", "parsers_detected": ["pixelogic"],
            "audio_count": 0, "subtitle_count": 0, "labels": [],
            "jar_inventory": ["foo.bin"],
        });
        let got = json!({
            "parser": "pixelogic", "parsers_detected": ["pixelogic"],
            "audio_count": 0, "subtitle_count": 0, "labels": [],
            "jar_inventory": ["foo.bin", "bar.xml", "extra.txt"],
        });
        assert!(
            diff_structural(&want, &got).is_empty(),
            "structural diff should ignore jar_inventory"
        );
    }

    #[test]
    fn diff_structural_catches_parser_change() {
        let want = json!({
            "parser": "pixelogic", "parsers_detected": ["pixelogic"],
            "audio_count": 0, "subtitle_count": 0, "labels": [],
        });
        let got = json!({
            "parser": "criterion", "parsers_detected": ["criterion"],
            "audio_count": 0, "subtitle_count": 0, "labels": [],
        });
        let diffs = diff_structural(&want, &got);
        assert!(diffs.iter().any(|d| d.starts_with("parser:")));
        assert!(diffs.iter().any(|d| d.starts_with("parsers_detected:")));
    }

    #[test]
    fn diff_structural_catches_label_count_change() {
        let want = json!({
            "parser": "p", "parsers_detected": ["p"],
            "audio_count": 2, "subtitle_count": 0,
            "labels": [
                {"stream_number": 1, "stream_type": "audio", "language": "eng", "name": "",
                 "codec_hint": "", "variant": "", "purpose": "Normal", "qualifier": "None"},
                {"stream_number": 2, "stream_type": "audio", "language": "spa", "name": "",
                 "codec_hint": "", "variant": "", "purpose": "Normal", "qualifier": "None"},
            ],
        });
        let got = json!({
            "parser": "p", "parsers_detected": ["p"],
            "audio_count": 1, "subtitle_count": 0,
            "labels": [
                {"stream_number": 1, "stream_type": "audio", "language": "eng", "name": "",
                 "codec_hint": "", "variant": "", "purpose": "Normal", "qualifier": "None"},
            ],
        });
        let diffs = diff_structural(&want, &got);
        assert!(diffs.iter().any(|d| d.contains("audio_count")));
        assert!(diffs.iter().any(|d| d.contains("labels.len")));
    }

    #[test]
    fn diff_structural_catches_language_drift() {
        // Same parser, same counts, but a language regressed.
        let want = json!({
            "parser": "p", "parsers_detected": ["p"],
            "audio_count": 1, "subtitle_count": 0,
            "labels": [
                {"stream_number": 1, "stream_type": "audio", "language": "eng", "name": "",
                 "codec_hint": "", "variant": "", "purpose": "Normal", "qualifier": "None"},
            ],
        });
        let got = json!({
            "parser": "p", "parsers_detected": ["p"],
            "audio_count": 1, "subtitle_count": 0,
            "labels": [
                {"stream_number": 1, "stream_type": "audio", "language": "", "name": "",
                 "codec_hint": "", "variant": "", "purpose": "Normal", "qualifier": "None"},
            ],
        });
        let diffs = diff_structural(&want, &got);
        assert!(
            diffs.iter().any(|d| d.contains("labels[0].language")),
            "expected language drift to be caught, got: {:?}",
            diffs
        );
    }

    #[test]
    fn diff_structural_normalizes_label_order() {
        // Same labels in different order — should compare equal once
        // sorted by (stream_type, stream_number).
        let want = json!({
            "parser": "p", "parsers_detected": ["p"],
            "audio_count": 2, "subtitle_count": 0,
            "labels": [
                {"stream_number": 1, "stream_type": "audio", "language": "eng", "name": "",
                 "codec_hint": "", "variant": "", "purpose": "Normal", "qualifier": "None"},
                {"stream_number": 2, "stream_type": "audio", "language": "spa", "name": "",
                 "codec_hint": "", "variant": "", "purpose": "Normal", "qualifier": "None"},
            ],
        });
        let got = json!({
            "parser": "p", "parsers_detected": ["p"],
            "audio_count": 2, "subtitle_count": 0,
            "labels": [
                {"stream_number": 2, "stream_type": "audio", "language": "spa", "name": "",
                 "codec_hint": "", "variant": "", "purpose": "Normal", "qualifier": "None"},
                {"stream_number": 1, "stream_type": "audio", "language": "eng", "name": "",
                 "codec_hint": "", "variant": "", "purpose": "Normal", "qualifier": "None"},
            ],
        });
        assert!(diff_structural(&want, &got).is_empty());
    }

    #[test]
    fn diff_exact_catches_jar_inventory_drift() {
        // What structural ignores, exact should catch.
        let want = json!({"foo": ["a"]});
        let got = json!({"foo": ["a", "b"]});
        assert!(!diff_exact(&want, &got).is_empty());
    }

    #[test]
    fn options_parse_minimal() {
        let opts = Options::parse(&["/path/to/corpus".to_string()]).unwrap();
        assert_eq!(opts.corpus_dir, PathBuf::from("/path/to/corpus"));
        assert!(!opts.update);
        assert!(!opts.exact);
        assert!(opts.filter.is_none());
    }

    #[test]
    fn options_parse_all_flags() {
        let opts = Options::parse(&[
            "/path".to_string(),
            "--update".to_string(),
            "--exact".to_string(),
            "--filter".to_string(),
            "disc-07".to_string(),
        ])
        .unwrap();
        assert!(opts.update);
        assert!(opts.exact);
        assert_eq!(opts.filter.as_deref(), Some("disc-07"));
    }

    #[test]
    fn options_parse_filter_without_value_errors() {
        assert!(Options::parse(&["/p".to_string(), "--filter".to_string()]).is_err());
    }
}
