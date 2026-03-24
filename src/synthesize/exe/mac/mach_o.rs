use bitflags::bitflags;
use bytemuck::NoUninit;

// Mach-O header files found here: /Library/Developer/CommandLineTools/SDKs/MacOSX.sdk/usr/include/mach-o

/// The Mach-O header, located at the top
#[repr(C)]
#[derive(NoUninit, Copy, Clone)]
pub struct Header {
    /// A magic number telling the OS this is a Mach-O file.
    /// Dependent on the architecture (32/64 bit) of the program.
    pub magic: Magic,

    pub cpu_type: CpuType,
    pub cpu_subtype: CpuSubtype,
    pub file_type: FileType,

    /// Number of load commands
    pub load_cmd_count: u32,

    /// Total size of load commands in bytes
    pub load_cmd_size: u32,

    pub flags: HeaderFlags,
    pub _reserved: u32,
}

#[repr(u32)]
#[derive(NoUninit, Copy, Clone)]
pub enum Magic {
    /// 32-bit architecture
    // X32 = 0xfeedface,

    /// 64-bit architecture
    X64 = 0xfeedfacf,
}

#[repr(i32)]
#[derive(NoUninit, Copy, Clone)]
pub enum CpuType {
    // Any = -1,
    // X86 = 7,
    // X86_64 = CpuType::X86 as i32 | 64,
    Arm64 = 0x0100000c,
}

#[repr(i32)]
#[derive(NoUninit, Copy, Clone)]
pub enum CpuSubtype {
    // LittleEndian = 0,
    // BigEndian = 1,
    Arm = 0,
    // X86 = 3,
}

#[repr(u32)]
#[derive(NoUninit, Copy, Clone)]
pub enum FileType {
    Execute = 2,
}

#[repr(transparent)]
#[derive(NoUninit, Copy, Clone)]
pub struct HeaderFlags(u32);

bitflags! {
    impl HeaderFlags: u32 {
        const NoUndefs              = 0b0000000000000000000001;
        const IncrLink              = 0b0000000000000000000010;
        const DyldLink              = 0b0000000000000000000100;
        const BinDatLoad            = 0b0000000000000000001000;
        const Prebound              = 0b0000000000000000010000;
        const SplitSegs             = 0b0000000000000000100000;
        const LazyInit              = 0b0000000000000001000000;
        const TwoLevel              = 0b0000000000000010000000;
        const ForceFlat             = 0b0000000000000100000000;
        const NoMultiDefs           = 0b0000000000001000000000;
        const NoFixPreBinding       = 0b0000000000010000000000;
        const PreBindable           = 0b0000000000100000000000;
        const AllModsBound          = 0b0000000001000000000000;
        const SubsectionsViaSymbols = 0b0000000010000000000000;
        const Canonical             = 0b0000000100000000000000;
        const WeakDefines           = 0b0000001000000000000000;
        const BindsToWeak           = 0b0000010000000000000000;
        const AllowStackExecution   = 0b0000100000000000000000;
        const RootSafe              = 0b0001000000000000000000;
        const SetUidSafe            = 0b0010000000000000000000;
        const NoReexportedDylibs    = 0b0100000000000000000000;
        const PIE                   = 0b1000000000000000000000;
    }
}

#[repr(u32)]
#[derive(NoUninit, Copy, Clone)]
pub enum LoadCommand {
    Segment = 0x19,
    EntryPoint = 0x80000028,
    LoadDyLinker = 0xe,
    CodeSignature = 0x1d,
    SymTab = 0x2,
    DySymTab = 0xb,
}

#[repr(C)]
#[derive(NoUninit, Copy, Clone)]
pub struct SegmentCommand {
    pub command: LoadCommand,
    pub command_size: u32,
    pub segment_name: [u8; 16],
    pub vmaddr: u64,
    pub vmsize: u64,
    pub file_offset: u64,
    pub file_size: u64,
    pub max_prot: MemoryPermissions,
    pub init_prot: MemoryPermissions,
    pub section_count: u32,
    pub flags: u32,
}

#[repr(transparent)]
#[derive(NoUninit, Copy, Clone)]
pub struct MemoryPermissions(i32);

bitflags! {
    impl MemoryPermissions: i32 {
        const Read      = 0b001;
        const Write     = 0b010;
        const Execute   = 0b100;

        const ReadWrite = Self::Read.bits() | Self::Write.bits();
        const ReadExecute = Self::Read.bits() | Self::Execute.bits();
    }
}

#[repr(C)]
#[derive(NoUninit, Copy, Clone)]
pub struct SectionHeader {
    pub section_name: [u8; 16],
    pub segment_name: [u8; 16],
    pub addr: u64,
    pub size: u64,
    pub offset: u32,
    pub align: u32,
    pub reloff: u32,
    pub nreloc: u32,
    pub flags: u32,
    pub _reserved1: u32,
    pub _reserved2: u32,
    pub _reserved3: u32,
}

#[repr(C)]
#[derive(NoUninit, Copy, Clone)]
pub struct EntryPointCommand {
    pub command: LoadCommand,
    pub command_size: u32,
    pub main_offset: u64,
    pub stack_size: u64,
}

#[repr(C)]
#[derive(NoUninit, Copy, Clone)]
pub struct DyLinkerCommand {
    pub command: LoadCommand,
    pub command_size: u32,
    pub path_str_offset: u32,
}

pub const CSMAGIC_EMBEDDED_SIGNATURE: u32 = 0xfade0cc0;

#[repr(C)]
#[derive(NoUninit, Copy, Clone)]
pub struct LinkEditDataCommand {
    pub command: LoadCommand,
    pub command_size: u32,
    pub data_offset: u32,
    pub data_size: u32,
}

#[repr(C)]
#[derive(NoUninit, Clone, Copy)]
pub struct SymTabCommand {
    pub command: LoadCommand,
    pub command_size: u32,
    pub symoff: u32,
    pub nsyms: u32,
    pub stroff: u32,
    pub strsize: u32,
}

#[repr(C)]
#[derive(NoUninit, Copy, Clone)]
pub struct DySymTabCommand {
    pub command: LoadCommand,
    pub command_size: u32,
    pub ilocalsym: u32,
    pub nlocalsym: u32,
    pub iextdefsym: u32,
    pub nextdefsym: u32,
    pub iundefsym: u32,
    pub nundefsym: u32,
    pub tocoff: u32,
    pub ntoc: u32,
    pub modtaboff: u32,
    pub nmodtab: u32,
    pub extrefsymoff: u32,
    pub nextrefsyms: u32,
    pub indirectsymoff: u32,
    pub nindirectsyms: u32,
    pub extreloff: u32,
    pub nextrel: u32,
    pub locreloff: u32,
    pub nlocrel: u32,
}

#[repr(C)]
#[derive(NoUninit, Clone, Copy)]
pub struct NList {
    pub str_table_idx: u32,
    pub n_type: NListType,
    pub n_sect: u8,
    pub n_desc: u16,
    pub n_value: u64,
}

#[repr(u8)]
#[derive(NoUninit, Clone, Copy)]
pub enum NListType {
    Sect = 0xe,
}
