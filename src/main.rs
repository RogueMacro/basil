use std::{
    io,
    path::{Path, PathBuf},
    time::Instant,
};

use clap::{Parser, Subcommand};
use colored::Colorize;
use istind::{Compiler, synthesize::exe::mac::AppleExecutable};

#[derive(Parser)]
#[command(version)]
struct Cli {
    #[clap(subcommand)]
    command: Command,
}

#[derive(Debug, Subcommand)]
enum Command {
    Build {
        file: PathBuf,

        #[arg(long = "asm", help = "Show generated assembly")]
        asm: bool,
    },
    Run {
        file: PathBuf,
    },
}

fn main() -> Result<(), Error> {
    let args = Cli::parse();

    if let Err(err) = cli(args) {
        eprintln!("{} {}", "error:".bright_red().bold(), err);
    }

    Ok(())
}

fn cli(args: Cli) -> Result<(), Error> {
    match args.command {
        Command::Build { file, asm } => {
            build(&file, asm)?;
        }
        Command::Run { file } => {
            build_and_run(&file)?;
        }
    }

    Ok(())
}

fn build_and_run(file: &Path) -> Result<(), Error> {
    let exe = build(file, false)?;

    println!(
        "{:>12} `{}`",
        "Running".bright_green().bold(),
        exe.to_string_lossy(),
    );

    let status = std::process::Command::new(exe).status()?;
    std::process::exit(status.code().unwrap_or(-1));
}

fn build(file: &Path, asm: bool) -> Result<PathBuf, Error> {
    let Some(module) = file.file_stem() else {
        return Err(Error::InvalidFile);
    };

    println!(
        "{:>12} {}",
        "Compiling".bright_green().bold(),
        module.to_string_lossy(),
    );

    let compiler = Compiler::<AppleExecutable>::default();

    let out_path = istind::files::target_mod(module)?;

    let start = Instant::now();
    if let Err(error_count) = compiler.compile(file, &out_path) {
        return Err(Error::CompilationFailed(error_count));
    }
    let end = Instant::now();
    let dur = end - start;

    // println!(
    //     "\r{:>12} {} in {:.2}s",
    //     "Compiled".bright_green().bold(),
    //     module.to_string_lossy(),
    //     dur.as_secs_f32(),
    // );

    if asm {
        print_assembly(&out_path);
    }

    Ok(out_path)
}

#[derive(Debug, thiserror::Error)]
enum Error {
    #[error("path is not a compilable file")]
    InvalidFile,
    #[error("failed to compile due to {0} error(s)")]
    CompilationFailed(usize),
    #[error("io error")]
    Io(#[from] io::Error),
}

#[cfg(target_os = "macos")]
fn print_assembly(exe: &Path) {
    std::process::Command::new("otool")
        .arg("-vt")
        .arg(exe)
        .status()
        .expect("program failed");
}
