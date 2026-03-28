# Agent Instructions

This repository contains **basil**, a compiler written entirely in Rust, covering both frontend and backend stages.

## Architecture

The compiler is structured as a Rust library crate (`src/lib.rs`) with a thin CLI binary (`src/main.rs`).

### Frontend (`src/analyze/`)
- **Lexer** (`src/analyze/lex/`) – tokenizes source text into `Token`s (keywords, identifiers, numbers, operators, semicolons). Errors are reported via the `ariadne` crate.
- **Parser / AST** (`src/analyze/ast/`) – parses the token stream into an `AST` containing `Item`s (currently functions) made up of `Statement`s and `Expression`s.

### Intermediate Representation (`src/ir/`)
- `IR::generate(ast)` lowers the AST into an architecture-neutral IR, including lifetime analysis (`src/ir/lifetime.rs`) and code-gen helpers (`src/ir/codegen.rs`).

### Backend (`src/synthesize/`)
- **Assembler** (`src/synthesize/arch/`) – `Assemble` trait with an AArch64 (ARM64) implementation (`src/synthesize/arch/arm/`). Produces `MachineCode` (raw bytes + entry-point offset).
- **Executable writer** (`src/synthesize/exe/`) – `Executable` trait; current implementation targets **macOS** (Mach-O via `apple-codesign`) in `src/synthesize/exe/mac/`.

## Build & Test

```bash
# Build
cargo build

# Run tests
cargo test

# Run the compiler on a source file
cargo run -- <path/to/source>
```

## Language Specification

The language being compiled is documented in [`SPEC.md`](SPEC.md). Refer to it when implementing or modifying language features.

## Key Conventions

- Rust edition **2024** is used; `let`/`while let` chains are idiomatic here.
- Error reporting uses `ariadne` with `ColorGenerator` for coloured labels; always propagate errors as `ariadne::Report` wrapped in `crate::analyze::Error`.
- New language features should be added to the `ast` types first, then lowered through the IR, and finally emitted by the ARM assembler.
- Keep frontend, IR, and backend concerns cleanly separated; avoid cross-layer imports (e.g. the IR must not import `synthesize`).
- The CLI binary lives in `src/main.rs` and should stay minimal – compilation logic belongs in the library.
