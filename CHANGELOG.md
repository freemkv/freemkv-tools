# Changelog

## 0.13.43 (2026-04-29)

### Pass 1 transport-failure recovery loop

- Transport failure (USB bridge crash) no longer kills the entire rip.
- Autorip re-discovers the drive after USB re-enumeration and resumes
  from the mapfile. Up to 10 attempts.
- `Error::is_scsi_transport_failure()` now matches DiscRead with
  status 0xFF (bridge crash) in addition to ScsiError.
- DriveSession tracks device_path for re-discovery.

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
