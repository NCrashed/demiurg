//! CLI wrapper around [`demiurg_convert::convert`] — what a DCC exporter
//! actually runs.
//!
//! ```text
//! demiurg-convert hero.json -o hero.demiurg
//! demiurg-convert - --base ./meshes -o hero.rkc   # manifest on stdin
//! ```
//!
//! Everything goes to stderr and the exit code, so a caller can treat stdout
//! as empty and just read the status.

use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use demiurg_convert::{Output, convert};

const USAGE: &str = "\
demiurg-convert — build a .demiurg / .rkc from a JSON exchange manifest

USAGE:
    demiurg-convert <manifest.json|-> -o <out.demiurg|out.rkc> [--base <dir>]

OPTIONS:
    -o, --out <path>   Output file; the extension picks the format.
        --base <dir>   Directory `vox_file` paths resolve against.
                       Defaults to the manifest's own directory
                       (the working directory when reading stdin).
    -h, --help         Print this.
";

fn main() -> ExitCode {
    match run() {
        Ok(msg) => {
            eprintln!("{msg}");
            ExitCode::SUCCESS
        }
        Err(msg) => {
            eprintln!("demiurg-convert: {msg}");
            ExitCode::FAILURE
        }
    }
}

/// The whole CLI, with every failure as a message for `main` to print.
fn run() -> Result<String, String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || args.iter().any(|a| a == "-h" || a == "--help") {
        return Ok(USAGE.to_string());
    }

    let mut input: Option<String> = None;
    let mut out: Option<PathBuf> = None;
    let mut base: Option<PathBuf> = None;
    let mut i = 0;
    while i < args.len() {
        let a = args[i].as_str();
        // A lone `-` is the stdin input, not a flag.
        match a {
            "-o" | "--out" => {
                i += 1;
                out = Some(PathBuf::from(
                    args.get(i).ok_or("-o needs a path".to_string())?,
                ));
            }
            "--base" => {
                i += 1;
                base = Some(PathBuf::from(
                    args.get(i).ok_or("--base needs a directory".to_string())?,
                ));
            }
            _ if a.starts_with('-') && a != "-" => {
                return Err(format!("unknown option {a}\n\n{USAGE}"));
            }
            _ if input.is_none() => input = Some(a.to_string()),
            _ => return Err(format!("unexpected argument {a}\n\n{USAGE}")),
        }
        i += 1;
    }

    let input = input.ok_or_else(|| format!("no manifest given\n\n{USAGE}"))?;
    let out = out.ok_or_else(|| format!("no -o output given\n\n{USAGE}"))?;
    let kind = Output::from_path(&out)
        .ok_or_else(|| format!("{}: expected a .demiurg or .rkc output", out.display()))?;

    let (json, base_dir) = if input == "-" {
        let mut buf = Vec::new();
        std::io::stdin()
            .read_to_end(&mut buf)
            .map_err(|e| format!("read stdin: {e}"))?;
        (buf, base.unwrap_or_else(|| PathBuf::from(".")))
    } else {
        let path = Path::new(&input);
        let bytes = std::fs::read(path).map_err(|e| format!("read {input}: {e}"))?;
        let dir = base.unwrap_or_else(|| {
            path.parent()
                .filter(|p| !p.as_os_str().is_empty())
                .map_or_else(|| PathBuf::from("."), Path::to_path_buf)
        });
        (bytes, dir)
    };

    let done = convert(&json, &base_dir, kind).map_err(|e| e.to_string())?;
    std::fs::write(&out, &done.bytes).map_err(|e| format!("write {}: {e}", out.display()))?;
    Ok(format!(
        "wrote {} ({}, {} bytes)",
        out.display(),
        done.stats,
        done.bytes.len()
    ))
}
