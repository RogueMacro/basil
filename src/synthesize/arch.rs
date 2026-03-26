use crate::ir::IR;

pub mod arm;

pub struct UnfinishedCode<A: Assembler>(pub(self) A);

impl<A: Assembler> UnfinishedCode<A> {
    pub fn size(&self) -> usize {
        self.0.current_offset()
    }

    pub fn finalize(mut self, str_literal_offset: usize) -> MachineCode {
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
    fn assemble(ir: IR) -> UnfinishedCode<Self>;

    fn current_offset(&self) -> usize;

    fn into_machine_code(self, str_literal_offset: usize) -> MachineCode;
}
