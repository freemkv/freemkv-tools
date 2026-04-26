// freemkv-tools — operator/debug toolkit for libfreemkv
// AGPL-3.0 — freemkv project
//
// Subcommands:
//   dd      sg_dd-style raw sector reader using libfreemkv's Drive API
//
// All subcommands go through the same Drive::open / init / read path the rip
// pipeline uses, so observed behaviour is real production behaviour at the
// transport layer (not a parallel implementation that could drift).

mod dd;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        usage();
        std::process::exit(0);
    }
    match args[1].as_str() {
        "dd" => match dd::run(&args[2..]) {
            Ok(()) => {}
            Err(e) => {
                eprintln!("dd: {}", e);
                std::process::exit(1);
            }
        },
        "version" | "--version" | "-V" => println!("{}", env!("CARGO_PKG_VERSION")),
        "help" | "--help" | "-h" => usage(),
        other => {
            eprintln!("unknown subcommand: {}", other);
            usage();
            std::process::exit(2);
        }
    }
}

fn usage() {
    println!("freemkv-tools {}", env!("CARGO_PKG_VERSION"));
    println!();
    println!("operator + debug toolkit for libfreemkv (NOT for end users)");
    println!();
    println!("Subcommands:");
    println!("  dd       raw sector read via libfreemkv (sg_dd-like)");
    println!("  version  print crate version");
    println!("  help     this message");
    println!();
    println!("Run a subcommand with --help for its arguments.");
}
