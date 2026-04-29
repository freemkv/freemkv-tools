# Changelog

## 0.13.42 (2026-04-29)

### Fix: transport failure skips instead of aborting

- Transport failure (USB bridge crash) now skips the failed ECC block
  (marks NonTrimmed) and continues. 3 consecutive transport failures
  still abort the copy. Previously, a single transport failure killed
  the entire rip.

## 0.13.41 (2026-04-29)

### Debug logging for sector-0 regression diagnosis

- Add debug logging to Drive::read and Disc::copy first reads.
- No functional changes.

## 0.13.40 (2026-04-28)

### Sync release — picks up libfreemkv 0.13.39

- Picks up ECC-block sweep, mapfile-based recovery, transport-failure abort.
- No freemkv-tools functional changes.
