//! `freemkv-tools dd` — raw sector read via libfreemkv's Drive API.
//!
//! Mirrors `sg_dd`'s flag style for muscle memory but goes through
//! [`libfreemkv::Drive`] so what you measure is what the rip pipeline
//! sees: same `Drive::open` / `init` / `Drive::scsi_mut().execute()`
//! path. Use it to characterise drive behaviour empirically — vary
//! timeout, pause, block size at the SG_IO layer, observe per-CDB
//! results — then encode the findings back into libfreemkv constants.
//!
//! Args (sg_dd-style `key=value`, plus a few of our own):
//!   if=DEV         input device (`/dev/sg4` or `/dev/sr0`); resolved to sg
//!   of=PATH        output file; default `/dev/null`
//!   bs=N           sector size in bytes; default 2048 (BD/UHD); only 2048 supported today
//!   skip=LBA       starting LBA; default 0
//!   count=N        total sectors to attempt; default 256
//!   bpt=N          sectors per SCSI READ command; default 32 (matches our rip pipeline)
//!   timeout=MS     SG_IO timeout per CDB in milliseconds; default 30000
//!   pause=MS       sleep between commands in milliseconds; default 0
//!   unlock=0|1     run `Drive::init` to unlock LibreDrive firmware; default 1
//!   recovery=0|1   pass `recovery=true` to Drive::read (= 60 s timeout flag); default 0
//!   retries=N      hammer the SAME LBA N+1 times; each attempt counted as
//!                  a separate CDB; ignores count/bpt advancement. Use this to
//!                  reproduce a single bad-sector recovery scenario; default 0
//!   verbose=0..3   per-cmd logging; default 1 (one line per CDB)
//!
//! Examples:
//!   freemkv-tools dd if=/dev/sg4 skip=1000000 count=100
//!   freemkv-tools dd if=/dev/sg4 skip=12500000 count=2000 bpt=32 timeout=30000 pause=2000
//!   freemkv-tools dd if=/dev/sg4 skip=12500000 count=200 bpt=1 timeout=60000 verbose=2
//!
//! Output (verbose=1):
//!   [   0] lba=12500000 cnt=32 ok=32 elapsed=  4ms
//!   [   1] lba=12500032 cnt=32 ok=32 elapsed=  3ms
//!   [   2] lba=12500064 cnt=32 ERR sense=0/0 elapsed=37412ms
//!   ...
//!   summary: cdbs=8 ok=5 err=3 sectors_ok=160 elapsed=112.4s rate=2.85 MB/s

use libfreemkv::Drive;
use libfreemkv::scsi::DataDirection;
use std::fs::File;
use std::io::Write;
use std::path::Path;
use std::time::{Duration, Instant};

#[derive(Debug)]
struct Args {
    input: String,
    output: String,
    bs: usize,
    skip: u32,
    count: u32,
    bpt: u16,
    timeout_ms: u32,
    pause_ms: u64,
    unlock: bool,
    recovery: bool,
    retries: u32,
    verbose: u8,
}

impl Args {
    fn parse(argv: &[String]) -> Result<Self, String> {
        let mut a = Args {
            input: String::new(),
            output: "/dev/null".to_string(),
            bs: 2048,
            skip: 0,
            count: 256,
            bpt: 32,
            timeout_ms: 30_000,
            pause_ms: 0,
            unlock: true,
            recovery: false,
            retries: 0,
            verbose: 1,
        };
        for arg in argv {
            if arg == "--help" || arg == "-h" {
                println!("{}", help_text());
                std::process::exit(0);
            }
            let (k, v) = match arg.split_once('=') {
                Some(kv) => kv,
                None => return Err(format!("expected key=value: {}", arg)),
            };
            match k {
                "if" => a.input = v.to_string(),
                "of" => a.output = v.to_string(),
                "bs" => a.bs = v.parse().map_err(|_| format!("bad bs: {}", v))?,
                "skip" => a.skip = v.parse().map_err(|_| format!("bad skip: {}", v))?,
                "count" => a.count = v.parse().map_err(|_| format!("bad count: {}", v))?,
                "bpt" => a.bpt = v.parse().map_err(|_| format!("bad bpt: {}", v))?,
                "timeout" => {
                    a.timeout_ms = v.parse().map_err(|_| format!("bad timeout: {}", v))?
                }
                "pause" => a.pause_ms = v.parse().map_err(|_| format!("bad pause: {}", v))?,
                "unlock" => a.unlock = bool_flag(v).map_err(|_| format!("bad unlock: {}", v))?,
                "recovery" => {
                    a.recovery = bool_flag(v).map_err(|_| format!("bad recovery: {}", v))?
                }
                "retries" => {
                    a.retries = v.parse().map_err(|_| format!("bad retries: {}", v))?
                }
                "verbose" => {
                    a.verbose = v.parse().map_err(|_| format!("bad verbose: {}", v))?
                }
                _ => return Err(format!("unknown key: {}", k)),
            }
        }
        if a.input.is_empty() {
            return Err("if=DEV is required".into());
        }
        if a.bs != 2048 {
            return Err("only bs=2048 is supported (BD/UHD sector size)".into());
        }
        if a.bpt == 0 {
            return Err("bpt must be >= 1".into());
        }
        Ok(a)
    }
}

fn bool_flag(v: &str) -> Result<bool, ()> {
    match v {
        "0" | "false" | "no" | "off" => Ok(false),
        "1" | "true" | "yes" | "on" => Ok(true),
        _ => Err(()),
    }
}

fn help_text() -> &'static str {
    "freemkv-tools dd — raw sector reader via libfreemkv

Usage: freemkv-tools dd if=DEV [of=PATH] [skip=LBA] [count=N] [bpt=N]
                       [timeout=MS] [pause=MS] [unlock=0|1]
                       [recovery=0|1] [verbose=0..3]

  if=DEV          input device (/dev/sg* or /dev/sr*)
  of=PATH         output file; default /dev/null
  bs=2048         sector size; only 2048 supported
  skip=0          starting LBA
  count=256       total sectors
  bpt=32          sectors per SCSI READ command
  timeout=30000   per-CDB timeout (ms)
  pause=0         sleep between CDBs (ms)
  unlock=1        run Drive::init() to unlock LibreDrive
  recovery=0      pass recovery=true to Drive::read
  verbose=1       0=summary only; 1=per-CDB; 2=+sense; 3=+raw"
}

pub fn run(argv: &[String]) -> Result<(), String> {
    let args = Args::parse(argv)?;
    if args.verbose > 0 {
        eprintln!(
            "freemkv-tools dd: if={} of={} skip={} count={} bpt={} timeout={}ms pause={}ms unlock={} recovery={}",
            args.input,
            args.output,
            args.skip,
            args.count,
            args.bpt,
            args.timeout_ms,
            args.pause_ms,
            args.unlock,
            args.recovery
        );
    }

    let dev = Path::new(&args.input);
    let mut drive = Drive::open(dev).map_err(|e| format!("open: {}", e))?;
    if args.verbose > 0 {
        eprintln!(
            "  drive: {} {} {}",
            drive.drive_id.vendor_id.trim(),
            drive.drive_id.product_id.trim(),
            drive.drive_id.product_revision.trim()
        );
    }

    if args.unlock {
        match drive.init() {
            Ok(()) => {
                if args.verbose > 0 {
                    eprintln!("  unlock: ok");
                }
            }
            Err(e) => eprintln!("  unlock: {} (continuing — drive may not need unlock)", e),
        }
    }

    let mut out: Box<dyn Write> = if args.output == "/dev/null" {
        Box::new(std::io::sink())
    } else {
        Box::new(File::create(&args.output).map_err(|e| format!("create {}: {}", args.output, e))?)
    };

    let mut buf = vec![0u8; args.bpt as usize * args.bs];
    // retries > 0 → hammer the same LBA (retries+1) times. count/bpt advancement ignored.
    let retry_mode = args.retries > 0;
    let cdbs = if retry_mode {
        args.retries + 1
    } else {
        args.count.div_ceil(args.bpt as u32)
    };
    let mut sectors_done: u32 = 0;
    let mut ok_cdbs: u32 = 0;
    let mut err_cdbs: u32 = 0;
    let mut sectors_ok: u32 = 0;
    let total_t0 = Instant::now();

    for i in 0..cdbs {
        let (lba, count) = if retry_mode {
            // Always re-read the same LBA, count = bpt sectors.
            (args.skip, args.bpt as u32)
        } else {
            let l = args.skip + sectors_done;
            let remaining = args.count - sectors_done;
            (l, remaining.min(args.bpt as u32))
        };
        let count = count as u16;

        // READ_10 CDB
        let cdb = [
            0x28, // READ_10
            0x00,
            (lba >> 24) as u8,
            (lba >> 16) as u8,
            (lba >> 8) as u8,
            lba as u8,
            0x00,
            (count >> 8) as u8,
            count as u8,
            0x00,
        ];
        let bytes = count as usize * args.bs;
        let buf_slice = &mut buf[..bytes];

        let cdb_t0 = Instant::now();
        let res = drive.scsi_mut().execute(
            &cdb,
            DataDirection::FromDevice,
            buf_slice,
            args.timeout_ms,
        );
        let elapsed_ms = cdb_t0.elapsed().as_millis();

        match res {
            Ok(r) => {
                let _ = out.write_all(&buf_slice[..r.bytes_transferred]);
                ok_cdbs += 1;
                sectors_ok += count as u32;
                if args.verbose >= 1 {
                    println!(
                        "[{:5}] lba={:>10} cnt={:>3} ok={:>3} bytes={:>6} elapsed={:>6}ms",
                        i, lba, count, count, r.bytes_transferred, elapsed_ms
                    );
                }
            }
            Err(libfreemkv::Error::ScsiError {
                opcode,
                status,
                sense_key,
            }) => {
                err_cdbs += 1;
                if args.verbose >= 1 {
                    println!(
                        "[{:5}] lba={:>10} cnt={:>3} ERR opcode=0x{:02x} status=0x{:02x} sense_key={:>2} elapsed={:>6}ms",
                        i, lba, count, opcode, status, sense_key, elapsed_ms
                    );
                }
            }
            Err(e) => {
                err_cdbs += 1;
                if args.verbose >= 1 {
                    println!(
                        "[{:5}] lba={:>10} cnt={:>3} ERR {:?} elapsed={:>6}ms",
                        i, lba, count, e, elapsed_ms
                    );
                }
            }
        }

        if !retry_mode {
            sectors_done += count as u32;
        }
        if args.pause_ms > 0 && i + 1 < cdbs {
            std::thread::sleep(Duration::from_millis(args.pause_ms));
        }
    }

    let total_secs = total_t0.elapsed().as_secs_f64();
    let rate_mbs = if total_secs > 0.0 {
        (sectors_ok as f64 * args.bs as f64) / 1_048_576.0 / total_secs
    } else {
        0.0
    };
    println!(
        "summary: cdbs={} ok={} err={} sectors_ok={} sectors_attempted={} elapsed={:.1}s rate={:.2} MB/s",
        cdbs, ok_cdbs, err_cdbs, sectors_ok, args.count, total_secs, rate_mbs
    );
    Ok(())
}
