# basil

A hobby compiler for a custom language, written in Rust.

## About

basil is a personal project exploring compiler construction from scratch. It covers the full compilation pipeline:

- **Lexer** – tokenizes source text into a stream of tokens
- **Parser / AST** – parses tokens into an abstract syntax tree
- **IR** – lowers the AST into an architecture-neutral intermediate representation
- **Backend** – emits native machine code (currently targeting AArch64/macOS)

