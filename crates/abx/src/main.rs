use ab_reflect::parser;
use ab_reflect::tape::{format_arg, Tape};
use ab_reflect::vm::Vm;
use clap::Parser;
use std::fs;
use std::io::{self, Read};

#[derive(Parser)]
#[command(name = "abx", about = "A=B^Reflect interpreter")]
struct Cli {
    /// Source file (use `-` or omit for stdin)
    file: Option<String>,

    /// Arguments appended to the initial tape
    #[arg(trailing_var_arg = true)]
    args: Vec<String>,

    /// Maximum execution steps (overrides pragma)
    #[arg(long)]
    fuel: Option<u64>,

    /// Maximum tape size in megabytes (overrides pragma)
    #[arg(long)]
    tape_limit: Option<usize>,

    /// Suppress final tape output on halt
    #[arg(long)]
    no_final: bool,

    /// Print every tape mutation to stderr
    #[arg(long)]
    trace: bool,
}

fn main() {
    if let Err(e) = run() {
        eprintln!("{e}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), ab_reflect::error::Error> {
    let cli = Cli::parse();

    // Read source
    let source = match cli.file.as_deref() {
        None | Some("-") => {
            let mut buf = String::new();
            io::stdin().read_to_string(&mut buf)?;
            buf
        }
        Some(path) => fs::read_to_string(path)?,
    };

    // Parse
    let program = parser::parse(&source)?;

    // Resource limits: CLI overrides pragmas
    let fuel = cli.fuel.or(program.fuel).unwrap_or(u64::MAX);
    let tape_limit = cli
        .tape_limit
        .or(program.tape_limit)
        .unwrap_or(64)
        .saturating_mul(1024 * 1024);

    // Build initial tape from first rule's LHS
    let mut tape = Tape::new(program.initial_tape().unwrap_or(""));

    // Append CLI args via Safe Injection Protocol
    for arg in &cli.args {
        tape.push_str(&format_arg(arg));
    }

    // Create and run VM
    let mut vm = Vm::new(tape, program.rules, fuel, tape_limit);
    vm.trace = cli.trace;
    vm.run()?;

    // Final output
    if !cli.no_final {
        let decoded = ab_reflect::tape::decode_escapes(vm.tape.as_str())?;
        print!("{decoded}");
    }

    Ok(())
}
