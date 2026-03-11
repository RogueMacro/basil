use std::{marker::PhantomData, path::Path, rc::Rc};

use ariadne::Source;

use crate::{
    analyze::{ErrorVec, ast::parse::Parser, lex::Lexer, semantics},
    ir::IR,
    synthesize::{
        arch::{Assemble, MachineCode, arm::ArmAssembler},
        exe::Executable,
    },
};

pub mod analyze;
pub mod files;
pub mod ir;
pub mod synthesize;

#[derive(Default)]
pub struct Compiler<E: Executable> {
    _marker: PhantomData<E>,
}

impl<E: Executable> Compiler<E> {
    pub fn compile(&self, path: impl AsRef<Path>, out_path: impl AsRef<Path>) -> Result<(), usize> {
        let path = path.as_ref();
        let source = std::fs::read_to_string(path).unwrap();

        let source_name = Rc::new(
            path.file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or(String::from("unknown")),
        );

        let code = match self.compile_source(source_name.clone(), &source) {
            Ok(code) => code,
            Err(errors) => {
                errors.dump(source_name, &Source::from(source));
                return Err(errors.len());
            }
        };

        E::default()
            .with_binary_identifier(source_name.as_ref())
            .build(code, out_path);

        Ok(())
    }

    pub fn compile_source(&self, name: Rc<String>, source: &str) -> Result<MachineCode, ErrorVec> {
        let lexer = Lexer::new(name.clone(), source)?;

        let parser = Parser::new(name.clone(), lexer);

        let ast = parser.into_ast()?;

        for (lib, item) in ast.imports() {
            println!("import {} from {}", item, lib);
        }

        let ast = semantics::analyze(ast, name)?;

        let ir = IR::generate(ast);
        // println!("{}", ir);

        let code = ArmAssembler::assemble(ir);

        Ok(code)
    }
}
