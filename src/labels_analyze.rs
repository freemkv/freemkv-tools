//! `freemkv-tools labels-analyze` — diagnostic for the BD-J label
//! parsers in `libfreemkv`.
//!
//! Given a disc image (typically a 1 GB head-of-disc capture from
//! `freemkv-tools dd`), mounts the UDF filesystem and runs
//! `libfreemkv::labels::analyze` to report:
//!
//!   - which parser matched (paramount / criterion / pixelogic / ctrm),
//!     or `None` if the labels code would have fallen through to its
//!     codec-name fallback,
//!   - the inventory of files under `/BDMV/JAR/*/` (helps spot
//!     unrecognized parser-source files when no parser matched),
//!   - audio / subtitle counts plus a sample of the raw labels emitted.
//!
//! Output is JSON to stdout — pipe to a file per disc, then aggregate
//! into a corpus coverage report.
//!
//! Args:
//!   <path>    disc image file (1 GB head capture or full ISO)
//!
//! Examples:
//!   freemkv-tools labels-analyze disc-01.bin
//!   freemkv-tools labels-analyze /srv/autorip/labels-corpus/disc-07.bin > disc-07.json

use libfreemkv::FileSectorSource;
use libfreemkv::labels::{StreamLabelType, analyze, clpi_audit};
use libfreemkv::read_filesystem;
use std::path::Path;

pub fn run(argv: &[String]) -> Result<(), String> {
    if argv.is_empty() || argv[0] == "--help" || argv[0] == "-h" {
        println!("{}", help_text());
        return Ok(());
    }
    let path_str = &argv[0];
    let path = Path::new(path_str);
    let mut reader =
        FileSectorSource::open(path).map_err(|e| format!("open {}: {}", path_str, e))?;
    let udf = read_filesystem(&mut reader).map_err(|e| format!("udf read_filesystem: {:?}", e))?;
    let result = analyze(&mut reader, &udf);

    let audio_count = result
        .labels
        .iter()
        .filter(|l| l.stream_type == StreamLabelType::Audio)
        .count();
    let subtitle_count = result
        .labels
        .iter()
        .filter(|l| l.stream_type == StreamLabelType::Subtitle)
        .count();

    // Raw codes (codec_hint + variant) seen across labels — for triaging
    // vocab gaps. Deduped, preserves first-seen order.
    let mut raw_codes: Vec<String> = Vec::new();
    for l in &result.labels {
        for code in [&l.codec_hint, &l.variant] {
            if !code.is_empty() && !raw_codes.contains(code) {
                raw_codes.push(code.clone());
            }
        }
    }

    let labels_json: Vec<serde_json::Value> = result
        .labels
        .iter()
        .map(|l| {
            serde_json::json!({
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

    // bdmt disc-level metadata: localized titles, disc-set position.
    // Orthogonal to per-stream labels; always surface when present.
    let disc_metadata_json = result.disc_metadata.as_ref().map(|m| {
        serde_json::json!({
            "titles": m.titles,
            "descriptions": m.descriptions,
            "disc_number": m.disc_number.map(|(n, total)| serde_json::json!([n, total])),
        })
    });

    // Per-playlist chapter summary (chapter count + approximate
    // duration in seconds, sourced from MPLS PlaylistMark entries).
    // Useful for identifying the "main feature" playlist at a glance
    // — typically the one with the longest duration.
    let chapters_json: Vec<_> = result
        .chapter_summary
        .iter()
        .map(|c| {
            serde_json::json!({
                "playlist": c.playlist,
                "chapter_count": c.chapter_count,
                "duration_secs": c.duration_secs,
            })
        })
        .collect();

    // CLPI vs MPLS cross-validation audit. Diagnostic only — purely
    // surfaces "do these two data sources agree on what's on this
    // disc?" for the user. Doesn't affect the labels output.
    let audit = clpi_audit::audit(&mut reader, &udf);
    let (clpi_only, mpls_only, matches, divergent) = audit.class_counts();
    let audit_json = serde_json::json!({
        "matches": matches,
        "clpi_only": clpi_only,
        "mpls_only": mpls_only,
        "divergent": divergent,
        "total_pids": audit.rows.len(),
    });

    let out = serde_json::json!({
        "image_path": path_str,
        "parser": result.parser,
        "confidence": result.confidence.map(|c| format!("{:?}", c)),
        "parsers_detected": result.parsers_detected,
        "audio_count": audio_count,
        "subtitle_count": subtitle_count,
        "jar_inventory": result.jar_inventory,
        "raw_codes": raw_codes,
        "labels": labels_json,
        "disc_metadata": disc_metadata_json,
        "gap_fill_added": result.gap_fill_added,
        "chapter_summary": chapters_json,
        "clpi_vs_mpls_audit": audit_json,
    });
    println!(
        "{}",
        serde_json::to_string_pretty(&out).map_err(|e| e.to_string())?
    );
    Ok(())
}

fn help_text() -> &'static str {
    "freemkv-tools labels-analyze — diagnostic for the libfreemkv label parsers

Usage: freemkv-tools labels-analyze <image>

  <image>   disc image file (1 GB head-of-disc capture, full ISO,
            or any file libfreemkv's FileSectorSource can mount)

Output: JSON to stdout. Fields: parser (matched parser name or null),
audio_count, subtitle_count, jar_inventory (filenames seen under
/BDMV/JAR/*/), raw_codes (unique codec/variant strings), labels (per-
stream detail).

Pipe to a per-disc file then aggregate into a corpus coverage report:
  freemkv-tools labels-analyze disc-07.bin > disc-07.json"
}
