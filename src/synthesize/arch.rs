use crate::ir::IR;

// mod _arm;
pub mod arm;

pub struct LinkableCode<A: Assembler>(pub(self) A);

impl<A: Assembler> LinkableCode<A> {
    pub fn size(&self) -> usize {
        self.0.code_size()
    }

    pub fn link(self, str_literal_offset: usize) -> MachineCode {
        self.0.into_machine_code(str_literal_offset)
    }
}

#[derive(Default)]
pub struct MachineCode {
    pub instructions: Vec<u8>,
    pub entry_point_offset: u64,
    pub symbols: Vec<(String, u64)>,
    pub str_literals: Vec<String>,
}

pub trait Assembler: Sized {
    fn assemble(ir: IR) -> LinkableCode<Self>;

    fn code_size(&self) -> usize;

    fn into_machine_code(self, str_literal_offset: usize) -> MachineCode;
}
