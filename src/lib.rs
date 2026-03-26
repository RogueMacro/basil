use std::{
    collections::HashMap,
    fs,
    marker::PhantomData,
    path::{Path, PathBuf},
    rc::Rc,
};

use ariadne::{Cache, FileCache, Source};

use crate::{
    analyze::{
        ErrorVec,
        ast::{AST, parse::Parser},
        lex::Lexer,
        semantics,
    },
    ir::IR,
    synthesize::{
        arch::{Assembler, MachineCode, UnfinishedCode, arm::ArmAssembler},
        exe::Executable,
    },
};

pub mod analyze;
pub mod files;
pub mod ir;
pub mod synthesize;

#[derive(Default)]
pub struct Compiler<E: Executable, A: Assembler> {
    _marker: PhantomData<(E, A)>,
}

impl<E: Executable, A: Assembler> Compiler<E, A> {
    pub fn compile(
        self,
        path: impl Into<PathBuf>,
        out_path: impl AsRef<Path>,
    ) -> Result<(), usize> {
        let path: Rc<PathBuf> = Rc::from(path.into());
        let source = fs::read_to_string(path.as_ref()).unwrap();

        let code = match self.compile_source(path.clone(), &source) {
            Ok(code) => code,
            Err(errors) => {
                errors.dump();
                return Err(errors.len());
            }
        };

        E::default()
            .with_binary_identifier("dirthouse")
            .build(code, out_path);

        Ok(())
    }

    pub fn compile_source(
        &self,
        name: Rc<PathBuf>,
        source: &str,
    ) -> Result<UnfinishedCode<A>, ErrorVec> {
        let mut ast = load_ast(name.clone(), source)?;

        let mut libmap = HashMap::new();
        for lib in ast.imports() {
            load_lib_recursive(lib, &mut libmap)?;
        }

        for lib_ast in libmap.into_values() {
            ast.items.extend(lib_ast.items);
        }

        let ast = semantics::analyze(ast)?;

        let ir = IR::generate(ast);
        println!("{}", ir);

        let code = A::assemble(ir);

        Ok(code)
    }
}

fn load_ast(name: Rc<PathBuf>, source: &str) -> Result<AST, ErrorVec> {
    let lexer = Lexer::new(name.clone(), source)?;
    let parser = Parser::new(name, lexer);
    let ast = parser.into_ast()?;

    Ok(ast)
}

fn load_lib_recursive(lib: &str, map: &mut HashMap<String, AST>) -> Result<(), ErrorVec> {
    if lib == "std"
        && !map.contains_key(lib)
        && let Ok(source) = fs::read_to_string(files::stdlib())
    {
        // it's ok if file doesn't exist. semantic analysis will flag it.
        let source_name = Rc::new(files::stdlib());
        let mut ast = load_ast(source_name, &source)?;
        ast.mangle(lib);
        map.insert(String::from("std"), ast);
    } else {
        todo!()
    }

    Ok(())
}
