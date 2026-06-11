use ab_reflect::parser;
use ab_reflect::tape::{format_arg, Tape};
use ab_reflect::vm::Vm;
use clap::Parser;
use serde::Deserialize;
use std::fs;
use std::io::{self, Read};

#[derive(Parser)]
#[command(name = "abx", about = "A=B^Reflect interpreter")]
struct Cli {
    /// Source file (use `-` or omit for stdin; `.abx` and `.abx.txt` supported)
    file: Option<String>,

    /// Arguments appended to the initial tape
    #[arg(trailing_var_arg = true)]
    args: Vec<String>,

    /// Maximum execution steps (overrides pragma and config)
    #[arg(long)]
    fuel: Option<u64>,

    /// Maximum tape size in megabytes (overrides pragma and config)
    #[arg(long)]
    tape_limit: Option<usize>,

    /// Suppress final tape output on halt (overrides config)
    #[arg(long)]
    no_final: bool,

    /// Print every tape mutation to stderr
    #[arg(long)]
    trace: bool,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "kebab-case")]
struct AbxConfig {
    fuel: Option<u64>,
    tape_limit: Option<usize>,
    no_final: Option<bool>,
}

fn main() {
    if let Err(e) = run() {
        eprintln!("{e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), ab_reflect::error::Error> {
    let cli = Cli::parse();

    // Load config file (abx.toml in current directory)
    let config = load_config();

    // Read source
    let source = match cli.file.as_deref() {
        None | Some("-") => {
            let mut buf = String::new();
            io::stdin().read_to_string(&mut buf)?;
            buf
        }
        Some(path) => {
            let resolved = resolve_source_path(path).ok_or_else(|| {
                io::Error::new(
                    io::ErrorKind::NotFound,
                    format!("source not found: {path}"),
                )
            })?;
            fs::read_to_string(resolved)?
        }
    };

    // Parse
    let program = parser::parse(&source)?;

    // Priority: CLI flags > pragma > config file > default
    let fuel = cli
        .fuel
        .or(program.fuel)
        .or(config.fuel)
        .unwrap_or(5_000_000);

    let tape_limit_mb = cli
        .tape_limit
        .or(program.tape_limit)
        .or(config.tape_limit)
        .unwrap_or(64);
    let tape_limit = tape_limit_mb.saturating_mul(1024 * 1024);

    let no_final = cli.no_final || config.no_final.unwrap_or(false);

    // Set up tracing subscriber
    let level = if cli.trace {
        tracing::Level::INFO
    } else {
        tracing::Level::WARN
    };
    tracing_subscriber::fmt()
        .with_max_level(level)
        .with_writer(std::io::stderr)
        .without_time()
        .with_target(false)
        .with_level(false)
        .init();

    // Build initial tape from first rule's LHS
    let mut tape = Tape::new(program.initial_tape().unwrap_or(""));

    // Append CLI args via Safe Injection Protocol
    for arg in &cli.args {
        tape.push_str(&format_arg(arg));
    }

    // Create and run VM
    let mut vm = Vm::new(tape, program.rules, fuel, tape_limit);
    vm.run()?;

    // Final output
    if !no_final {
        let decoded = ab_reflect::tape::decode_escapes(vm.tape.as_str())?;
        print!("{decoded}");
    }

    Ok(())
}

/// Load `abx.toml` from the current working directory, if present.
fn load_config() -> AbxConfig {
    match fs::read_to_string("abx.toml") {
        Ok(content) => toml::from_str(&content).unwrap_or_default(),
        Err(_) => AbxConfig::default(),
    }
}

/// Resolve a source file path, trying `.abx` and `.abx.txt` extensions.
fn resolve_source_path(path: &str) -> Option<String> {
    if fs::metadata(path).map(|m| m.is_file()).unwrap_or(false) {
        return Some(path.to_string());
    }

    let with_abx = format!("{path}.abx");
    if fs::metadata(&with_abx).map(|m| m.is_file()).unwrap_or(false) {
        return Some(with_abx);
    }

    let with_abx_txt = format!("{path}.abx.txt");
    if fs::metadata(&with_abx_txt).map(|m| m.is_file()).unwrap_or(false) {
        return Some(with_abx_txt);
    }

    None
}
