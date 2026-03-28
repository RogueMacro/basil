use std::{fs, path::Path, rc::Rc};

use basil::{
    Compiler,
    synthesize::exe::{DummyExecutable, Executable, mac::AppleExecutable},
};

fn mod_main() -> Rc<String> {
    Rc::new(String::from("main"))
}

fn compiles(source: &str) {
    let compiler: Compiler<DummyExecutable> = Compiler::default();
    assert!(compiler.compile_source(mod_main(), source).is_ok());
}

fn fails(source: &str) {
    let compiler: Compiler<DummyExecutable> = Compiler::default();
    assert!(compiler.compile_source(mod_main(), source).is_err());
}

fn runs(test_name: &str, expect_exit_code: i32, source: &str) {
    let compiler: Compiler<DummyExecutable> = Compiler::default();
    let code = compiler.compile_source(mod_main(), source).unwrap();

    fs::create_dir_all("ctarget/test").unwrap();

    let mut exe = AppleExecutable::default().with_binary_identifier("basil.test");
    exe.build(code, Path::new("ctarget/test").join(test_name));
    let status = exe.run().unwrap();

    assert_eq!(status.code(), Some(expect_exit_code));
}

#[test]
fn minimal_implicit() {
    fails("fn main() {}");
}

#[test]
fn minimal_return() {
    runs(
        "minimal_return",
        0,
        "
        fn main() {
            return 0;
        }
        ",
    );
}

#[test]
fn minimal_return1() {
    runs(
        "minimal_return1",
        1,
        "
        fn main() {
            return 1;
        }
        ",
    );
}

#[test]
fn assignment() {
    runs(
        "assignment",
        2,
        "
        fn main() {
            a := 2;
            return a;
        }
        ",
    );
}

#[test]
fn addition() {
    runs(
        "addition",
        5,
        "
        fn main() {
            a := 2;
            b := 3;
            return a + b;
        }
        ",
    );
}
