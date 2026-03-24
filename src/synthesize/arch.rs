use crate::ir::IR;

pub mod arm;

#[derive(Default)]
pub struct MachineCode {
    pub instructions: Vec<u8>,
    pub entry_point_offset: u64,
    pub symbols: Vec<(String, u64)>,
}

pub trait Assemble {
    fn assemble(ir: IR) -> MachineCode;
}
