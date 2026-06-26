use std::{
    fs,
    path::{Path, PathBuf},
    rc::Rc,
};

use basil::{
    Compiler,
    analyze::{ast::parse::Parser, lex::Lexer},
    synthesize::exe::{DummyExecutable, Executable, mac::AppleExecutable},
};

// fn compiles(source: &str) {
//     let compiler: Compiler<DummyExecutable> = Compiler::default();
//     assert!(compiler.compile_source(mod_main(), source).is_ok());
// }
//
// fn fails(source: &str) {
//     let compiler: Compiler<DummyExecutable> = Compiler::default();
//     assert!(compiler.compile_source(mod_main(), source).is_err());
// }

// fn runs(test_name: &str, expect_exit_code: i32, source: &str) {
//     let compiler: Compiler<DummyExecutable> = Compiler::default();
//     let code = compiler.compile_source(mod_main(), source).unwrap();
//
//     fs::create_dir_all("ctarget/test").unwrap();
//
//     let mut exe = AppleExecutable::default().with_binary_identifier("basil.test");
//     exe.build(code, Path::new("ctarget/test").join(test_name));
//     let status = exe.run().unwrap();
//
//     assert_eq!(status.code(), Some(expect_exit_code));
// }
