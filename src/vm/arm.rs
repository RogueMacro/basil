use std::{
    cell::{Cell, RefCell},
    collections::{BTreeMap, HashMap},
    fmt::Write,
    io,
    time::Duration,
};

use crossterm::{
    ExecutableCommand,
    event::{self, Event, KeyCode, KeyEvent, KeyEventKind, MouseEventKind},
};
use ratatui::{
    DefaultTerminal, Frame,
    buffer::Buffer,
    layout::{Constraint, Direction, Layout, Rect},
    style::{Color, Style, Stylize},
    symbols::border,
    text::{Line, Text},
    widgets::{
        Block, Padding, Paragraph, Scrollbar, ScrollbarOrientation, ScrollbarState, StatefulWidget,
        Widget,
    },
};
use strum::IntoEnumIterator;

use crate::{
    ir::{Condition, StrId, VregSize},
    synthesize::arch::{
        LinkableCode,
        arm::{
            ArmAssembler,
            instr::{AddImmVal, BranchOffset, EitherOffset, FnOffset, ImmShift16, PageAddr},
            reg::Register,
        },
    },
};

const MEGABYTE: usize = 1_000_000;
const STACK_SIZE: usize = MEGABYTE;
const TOTAL_MEMORY: usize = 5 * MEGABYTE;
const RESERVED_BSS: usize = 1_000;

type Inst = crate::synthesize::arch::arm::instr::Inst<Register>;

pub fn debug_tui(code: LinkableCode<ArmAssembler>) {
    let asm = code.unwrap_assembler();
    // println!("code: {}", asm.code.);

    let instructions = asm.instructions;
    // let instructions = Vec::from_iter([
    //     Inst::Movz {
    //         shift: ImmShift16::L0,
    //         value: 11,
    //         dest: Register::X1,
    //     },
    //     Inst::Movz {
    //         shift: ImmShift16::L0,
    //         value: 22,
    //         dest: Register::X2,
    //     },
    //     Inst::Add {
    //         a: Register::X1,
    //         b: Register::X0,
    //         dest: Register::X0,
    //     },
    //     Inst::Branch {
    //         offset: BranchOffset::Fixed(-3),
    //     },
    // ]);

    let mut vm = VM::new(
        instructions,
        asm.code.str_literals,
        asm.str_literal_offsets,
        asm.code.entry_point_offset,
        asm.code.symbols,
    );

    ratatui::run(|terminal| vm.run(terminal)).expect("ratatui app failed");
}

#[derive(PartialEq)]
enum DebugState {
    Running,
    Paused,
    Quit,
}

struct VM {
    instructions: Vec<Inst>,
    reg_snapshot: BTreeMap<Register, u64>,
    registers: BTreeMap<Register, u64>,
    symbols: HashMap<String, u64>,
    memory: Vec<u8>,
    compares: (u64, u64),
    mmap_ptr: u64,

    inst_text: Vec<Line<'static>>,
    output: RefCell<Vec<Line<'static>>>,
    max_output_lines: Cell<usize>,
    stack_scroll: usize,
    stack_scroll_state: ScrollbarState,
    stack_max_scroll: usize,

    current_inst: usize,
    inst_per_sec: u64,

    state: DebugState,
}

impl VM {
    pub fn new(
        instructions: Vec<Inst>,
        strings: Vec<(usize, String)>,
        str_offsets: HashMap<StrId, usize>,
        entry_point: u64,
        symbols: Vec<(String, u64)>,
    ) -> Self {
        let inst_text = instructions
            .iter()
            .enumerate()
            .map(|(offset, inst)| {
                let offset = offset * 4;

                let symbol = symbols
                    .iter()
                    .find_map(|(s, o)| {
                        if *o == (offset as u64) {
                            Some(s.as_str())
                        } else {
                            None
                        }
                    })
                    .unwrap_or("");

                Line::from(format!(
                    "{:<20} 0x{:08x}  {}",
                    symbol,
                    offset,
                    display_inst(inst, offset, &strings, &str_offsets)
                ))
            })
            .collect();

        let mut memory = vec![0; TOTAL_MEMORY];
        let mut last_offset = 0;
        let mut end_of_strings = 0;
        for (offset, string) in strings {
            if offset != 0 {
                assert!(offset > last_offset);
                last_offset = offset;
            }

            let addr = STACK_SIZE + offset;
            end_of_strings = addr + string.len() + 1;

            memory.splice(addr..addr + string.len() + 1, string.into_bytes());
        }

        Self {
            instructions,
            reg_snapshot: Register::iter().map(|r| (r, 0)).collect(),
            registers: Register::iter().map(|r| (r, 0)).collect(),
            symbols: symbols.into_iter().collect(),
            memory,
            compares: (0, 0),
            mmap_ptr: end_of_strings as u64,

            inst_text,
            output: RefCell::new(Vec::new()),
            max_output_lines: Cell::new(0),
            stack_scroll: 0,
            stack_scroll_state: ScrollbarState::default(),
            stack_max_scroll: 0,

            current_inst: (entry_point / 4) as usize,
            inst_per_sec: 16,

            state: DebugState::Paused,
        }
    }

    pub fn run(&mut self, terminal: &mut DefaultTerminal) -> io::Result<()> {
        self.inst_text[self.current_inst].style = Style::default().fg(Color::Green).bold();

        *self.registers.get_mut(&Register::SP).unwrap() = STACK_SIZE as u64;
        *self.registers.get_mut(&Register::FP).unwrap() = STACK_SIZE as u64;

        std::io::stdout()
            .execute(crossterm::event::EnableMouseCapture)
            .unwrap();

        loop {
            terminal.draw(|frame| self.render(frame))?;
            self.handle_events()?;

            self.stack_scroll = self.stack_scroll.clamp(0, self.stack_max_scroll);
            self.stack_scroll_state = self.stack_scroll_state.position(self.stack_scroll);

            match self.state {
                DebugState::Running => {
                    self.execute_inst();
                }
                DebugState::Paused => {}
                DebugState::Quit => break,
            }
        }

        std::io::stdout()
            .execute(crossterm::event::DisableMouseCapture)
            .unwrap();

        Ok(())
    }

    fn handle_events(&mut self) -> io::Result<()> {
        let tick_rate = Duration::from_millis(1000 / self.inst_per_sec);

        if event::poll(tick_rate)? {
            match event::read()? {
                // it's important to check that the event is a key press event as
                // crossterm also emits key release and repeat events on Windows.
                Event::Key(key_event) if key_event.kind == KeyEventKind::Press => {
                    self.handle_key_event(key_event)
                }
                Event::Mouse(mouse) => match mouse.kind {
                    MouseEventKind::ScrollUp => {
                        self.stack_scroll = self
                            .stack_scroll
                            .saturating_sub(1)
                            .clamp(0, self.stack_max_scroll);
                        self.stack_scroll_state =
                            self.stack_scroll_state.position(self.stack_scroll);
                    }
                    MouseEventKind::ScrollDown => {
                        self.stack_scroll = self
                            .stack_scroll
                            .saturating_add(1)
                            .clamp(0, self.stack_max_scroll);
                        self.stack_scroll_state =
                            self.stack_scroll_state.position(self.stack_scroll);
                    }
                    _ => {}
                },
                _ => {}
            }
        }

        Ok(())
    }

    fn handle_key_event(&mut self, key_event: KeyEvent) {
        match key_event.code {
            KeyCode::Char('q') => self.quit(),
            KeyCode::Char('n') => self.execute_inst(),
            KeyCode::Char('c') => self.state = DebugState::Running,
            KeyCode::Char('p') => self.state = DebugState::Paused,
            KeyCode::Char('i') => self.inst_per_sec = (self.inst_per_sec - 1).clamp(1, u64::MAX),
            KeyCode::Char('I') => self.inst_per_sec += 1,
            KeyCode::Char('m') => self.inst_per_sec = 1,
            KeyCode::Char('M') => self.inst_per_sec = 10000,
            _ => {}
        }
    }

    fn quit(&mut self) {
        self.state = DebugState::Quit;
    }
}

impl VM {
    fn render(&mut self, frame: &mut Frame) {
        let main_layout = Layout::default()
            .direction(Direction::Horizontal)
            .constraints(vec![Constraint::Percentage(40), Constraint::Percentage(60)])
            .split(frame.area());

        let right_side = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(main_layout[1]);

        let regs_and_stack = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(right_side[0]);

        let settings_and_output = Layout::default()
            .direction(Direction::Vertical)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(right_side[1]);

        self.render_inst(main_layout[0], frame);
        self.render_regs(regs_and_stack[0], frame);
        self.render_stack(regs_and_stack[1], frame);
        self.render_settings(settings_and_output[0], frame);
        self.render_output(settings_and_output[1], frame);
    }

    fn render_inst(&self, area: Rect, frame: &mut Frame) {
        let title = Line::from("Instructions".bold());
        let keybinds = Line::from(vec![
            " Next instruction ".into(),
            "<n>".blue().bold(),
            " Continue ".into(),
            "<c>".blue().bold(),
            " Pause ".into(),
            "<p>".blue().bold(),
            " Quit ".into(),
            "<q> ".blue().bold(),
        ]);

        let border_color = match self.state {
            DebugState::Running => Color::Blue,
            DebugState::Paused => Color::Yellow,
            DebugState::Quit => Color::Red,
        };

        let block = Block::bordered()
            .title(title.centered())
            .title_bottom(keybinds.centered())
            .padding(Padding::horizontal(2))
            .border_set(border::THICK)
            .border_style(Style::new().fg(border_color));

        let cur_inst = self.current_inst as u16;
        let scroll_y = cur_inst.saturating_sub(area.height / 2) + 5;
        // let scroll_y = scroll_y.clamp(0, cur_inst - area.height + 5);

        frame.render_widget(
            Paragraph::new(self.inst_text.clone())
                .left_aligned()
                .block(block)
                .scroll((scroll_y, 0)),
            area,
        );
    }

    fn render_regs(&self, area: Rect, frame: &mut Frame) {
        let title = Line::from("Registers".bold());

        let mut text_left = Vec::new();
        for (reg, val) in self.registers.iter() {
            let mut line = Line::from(format!("{:>3}: {:<#018x}", reg.to_string(), val));
            if self.reg_snapshot.get(reg).unwrap() != val {
                line = line.cyan();
            }
            text_left.push(line);
        }

        let block = Block::bordered()
            .title(title.centered())
            .border_set(border::THICK)
            .padding(Padding::new(0, 0, 1, 0));

        let split_layout = Layout::default()
            .direction(Direction::Horizontal)
            .constraints([Constraint::Percentage(50), Constraint::Percentage(50)])
            .split(block.inner(area));

        let text_right = text_left.split_off(16);

        frame.render_widget(Paragraph::new(text_left).centered(), split_layout[0]);
        frame.render_widget(Paragraph::new(text_right).centered(), split_layout[1]);
        frame.render_widget(block, area);
    }

    fn render_stack(&mut self, area: Rect, frame: &mut Frame) {
        let title = Line::from("Stack".bold());

        let fp = *self.registers.get(&Register::FP).unwrap() as usize;
        let sp = *self.registers.get(&Register::SP).unwrap() as usize;
        // let top = (sp + area.height as usize - 2).clamp(0, STACK_SIZE);
        let top = STACK_SIZE;

        let mut text = String::new();
        let mut linecount = 0usize;
        for offset in (sp..top).rev() {
            let prefix = if offset == sp {
                "sp"
            } else if offset == fp {
                "fp"
            } else {
                ""
            };

            if offset % 8 == 0 {
                let dw_int =
                    u64::from_le_bytes(self.memory[offset..(offset + 8)].try_into().unwrap());
                writeln!(
                    &mut text,
                    "{:<3} {:#08x}: {:<20} {:<#20x}",
                    prefix, offset, self.memory[offset], dw_int
                )
                .unwrap();
            } else {
                writeln!(
                    &mut text,
                    "{:<3} {:#08x}: {:<41}",
                    prefix, offset, self.memory[offset]
                )
                .unwrap();
            }

            linecount += 1;
        }

        self.stack_max_scroll = linecount.saturating_sub(area.height as usize) + 2;
        self.stack_scroll_state = self
            .stack_scroll_state
            .content_length(self.stack_max_scroll);

        let block = Block::bordered()
            .title(title.centered())
            .border_set(border::THICK);

        frame.render_widget(
            Paragraph::new(text)
                .centered()
                .block(block)
                .scroll((self.stack_scroll as u16, 0)),
            area,
        );

        frame.render_stateful_widget(
            Scrollbar::new(ScrollbarOrientation::VerticalRight),
            area,
            &mut self.stack_scroll_state,
        );
    }

    fn render_settings(&self, area: Rect, frame: &mut Frame) {
        let title = Line::from("Settings".bold());

        let text = format!(
            "Speed: (i)- {} +(I) instructions/sec | min(m) max(M)",
            self.inst_per_sec
        );

        let block = Block::bordered()
            .title(title.centered())
            .border_set(border::THICK);

        frame.render_widget(Paragraph::new(text).centered().block(block), area);
    }

    fn render_output(&self, area: Rect, frame: &mut Frame) {
        self.max_output_lines.set(area.height as usize - 2);

        let title = Line::from("Output".bold());

        let block = Block::bordered()
            .title(title.centered())
            .border_set(border::THICK);

        frame.render_widget(
            Paragraph::new(self.output.borrow().clone())
                .centered()
                .block(block),
            area,
        );
    }
}

impl VM {
    fn write_output(&self, line: impl Into<String>) {
        let mut output = self.output.borrow_mut();

        if output.len() > self.max_output_lines.get() {
            output.remove(0);
        }

        output.push(Line::from(line.into()));
    }

    fn check_condition(&self, cond: Condition) -> bool {
        match cond {
            Condition::Equal => self.compares.0 == self.compares.1,
            Condition::NotEqual => self.compares.0 != self.compares.1,
            Condition::UnsignedGreaterOrEqual => self.compares.0 >= self.compares.1,
            Condition::UnsignedLess => self.compares.0 < self.compares.1,
            Condition::UnsignedGreater => self.compares.0 > self.compares.1,
            Condition::UnsignedLessOrEqual => self.compares.0 <= self.compares.1,
            Condition::SignedGreaterOrEqual => todo!(),
            Condition::SignedLess => todo!(),
            Condition::SignedGreater => todo!(),
            Condition::SignedLessOrEqual => todo!(),
            Condition::Negative => todo!(),
            Condition::PositiveOrZero => todo!(),
            Condition::Overflow => todo!(),
            Condition::NoOverflow => todo!(),
            Condition::Always => true,
            Condition::Never => false,
        }
    }

    fn execute_inst(&mut self) {
        if self.current_inst >= self.instructions.len() {
            self.write_output("no more instructions");
            return;
        }

        self.reg_snapshot = self.registers.clone();

        self.inst_text[self.current_inst].style = Style::reset();

        let mut is_branch = false;

        match self
            .instructions
            .get(self.current_inst)
            .expect("invalid instruction index")
        {
            Inst::Add { a, b, dest } => {
                let a = *self.registers.get(a).unwrap();
                let b = *self.registers.get(b).unwrap();
                *self.registers.get_mut(dest).unwrap() = a + b;
            }
            Inst::AddImm { a, imm, dest } => {
                let a = *self.registers.get(&a.expect_phys()).unwrap();
                let result = a + match imm {
                    AddImmVal::Fixed(u12) => Into::<u16>::into(*u12) as u64,
                    AddImmVal::String(offset) => (STACK_SIZE + *offset) as u64,
                    AddImmVal::Bss(offset) => *offset,
                };

                self.registers.insert(dest.expect_phys(), result);
            }
            Inst::Adrp { page_addr, dest } => match page_addr {
                PageAddr::Fixed(i21) => todo!(),
                PageAddr::String(offset) => {
                    //todo
                }
                PageAddr::Bss(offset) => {
                    //todo
                }
            },
            Inst::Branch { offset } => {
                is_branch = true;
                match offset {
                    BranchOffset::Fixed(offset) => {
                        self.current_inst = (self.current_inst as i32 + *offset) as usize
                    }
                    BranchOffset::Dynamic(label) => todo!(),
                }
            }
            Inst::BranchLink { offset } => match offset {
                FnOffset::Fixed(offset) => {
                    is_branch = true;
                    self.registers
                        .insert(Register::LR, (self.current_inst as u64 + 1) * 4);
                    self.current_inst = ((self.current_inst as i32) + *offset) as usize;
                }
                FnOffset::Dynamic(label) => match label.as_str() {
                    "std::syscall::mmap" => {
                        self.registers.insert(Register::X0, self.mmap_ptr);
                        self.mmap_ptr += 4096;
                    }
                    _ => {
                        is_branch = true;
                        self.registers
                            .insert(Register::LR, (self.current_inst as u64 + 1) * 4);
                        self.current_inst = *self.symbols.get(label).expect(label) as usize / 4
                    }
                },
            },
            Inst::BranchZero { offset, reg } => {
                is_branch = *self.registers.get(reg).unwrap() == 0;
                if is_branch {
                    match offset {
                        BranchOffset::Fixed(offset) => {
                            self.current_inst = (self.current_inst as i32 + *offset) as usize
                        }
                        BranchOffset::Dynamic(label) => todo!(),
                    }
                }
            }
            Inst::Cmp { a, b } => {
                let a = *self.registers.get(a).unwrap();
                let b = *self.registers.get(b).unwrap();
                self.compares = (a, b);
            }
            Inst::CmpImm { val, imm } => {
                let a = *self.registers.get(val).unwrap();
                let b = Into::<i16>::into(*imm) as u64;
                self.compares = (a, b);
            }
            Inst::Cset { inv_cond, dest } => {
                let value = if self.check_condition(inv_cond.inverted()) {
                    1
                } else {
                    0
                };

                self.registers.insert(*dest, value);
            }
            Inst::Load {
                base,
                offset,
                dest,
                size,
            } => {
                let base = *self.registers.get(&base.expect_phys()).unwrap();
                let offset = size.to_bytes()
                    * match offset {
                        EitherOffset::Reg(either_reg) => {
                            *self.registers.get(&either_reg.expect_phys()).unwrap()
                        }
                        EitherOffset::Imm(u12) => Into::<u16>::into(*u12) as u64,
                    };

                let addr = (base + offset) as usize;
                let bytes = &self.memory[addr..(addr + size.to_bytes() as usize)];
                let value = match size {
                    VregSize::Byte => u8::from_le_bytes(bytes.try_into().unwrap()) as u64,
                    VregSize::Halfword => u16::from_le_bytes(bytes.try_into().unwrap()) as u64,
                    VregSize::Word => u32::from_le_bytes(bytes.try_into().unwrap()) as u64,
                    VregSize::Doubleword => u64::from_le_bytes(bytes.try_into().unwrap()),
                    VregSize::Bytes(bytes) => panic!(),
                };

                self.registers.insert(*dest, value);
            }
            Inst::LoadPair {
                base,
                first,
                second,
                offset,
            } => {
                let addr = *self.registers.get(base).unwrap() as usize;
                // let offset = Into::<i8>::into(*offset) as u64;

                let a = u64::from_le_bytes((&self.memory[addr..addr + 8]).try_into().unwrap());
                let b = u64::from_le_bytes((&self.memory[addr + 8..addr + 16]).try_into().unwrap());
                self.registers.insert(*first, a);
                self.registers.insert(*second, b);
                self.registers.insert(*base, addr as u64 + 16);
            }
            Inst::Mov { src, dest } => {
                self.registers.insert(
                    dest.expect_phys(),
                    *self.registers.get(&src.expect_phys()).unwrap(),
                );
            }
            Inst::Movk { shift, value, dest } => {
                let shift_amount = match shift {
                    ImmShift16::L0 => 0,
                    ImmShift16::L16 => 16,
                    ImmShift16::L32 => 32,
                    ImmShift16::L48 => 48,
                };

                let value = *value as u64;
                let dest_value = *self.registers.get(dest).unwrap();

                let clear_mask = 15 << shift_amount;
                let result = (dest_value ^ clear_mask) | (value << shift_amount);

                self.registers.insert(*dest, result);
            }
            Inst::Movz { shift, value, dest } => {
                let value = *value as u64;
                let value = match shift {
                    ImmShift16::L0 => value,
                    ImmShift16::L16 => value << 16,
                    ImmShift16::L32 => value << 32,
                    ImmShift16::L48 => value << 48,
                };

                *self.registers.get_mut(dest).unwrap() = value;
            }
            Inst::Mul { a, b, dest } => {
                let a = *self.registers.get(a).unwrap();
                let b = *self.registers.get(b).unwrap();
                self.registers.insert(*dest, a * b);
            }
            Inst::Ret { .. } => {
                is_branch = true;
                let jump_to = *self.registers.get(&Register::LR).unwrap();
                self.current_inst = jump_to as usize / 4;
            }
            Inst::Store {
                src: source,
                base,
                offset,
                size,
            } => {
                let value = *self.registers.get(source).unwrap();
                let base = *self.registers.get(&base.expect_phys()).unwrap();
                let offset = match offset {
                    EitherOffset::Reg(reg) => *self.registers.get(&reg.expect_phys()).unwrap(),
                    EitherOffset::Imm(u12) => Into::<u16>::into(*u12) as u64,
                };

                let offset = (base + offset) as usize;

                let bytes = match size {
                    VregSize::Byte => vec![value as u8],
                    VregSize::Halfword => (value as u16).to_le_bytes().to_vec(),
                    VregSize::Word => (value as u32).to_le_bytes().to_vec(),
                    VregSize::Doubleword => value.to_le_bytes().to_vec(),
                    _ => todo!(),
                };

                self.memory
                    .splice(offset..(offset + size.to_bytes() as usize), bytes);
            }
            Inst::StorePair {
                base,
                first,
                second,
                offset,
            } => {
                // This is always post-index
                let offset = Into::<i8>::into(*offset) as i64;
                let base_ref = self.registers.get_mut(base).unwrap();
                let new_offset = (*base_ref as i64 + offset * 8) as usize;
                *base_ref = new_offset as u64;

                let first_val = *self.registers.get(first).unwrap();
                let second_val = *self.registers.get(second).unwrap();

                self.memory
                    .splice(new_offset..(new_offset + 8), first_val.to_le_bytes());
                self.memory.splice(
                    (new_offset + 8)..(new_offset + 16),
                    second_val.to_le_bytes(),
                );
            }
            Inst::Sub { a, b, dest } => {
                let a = *self.registers.get(a).unwrap();
                let b = *self.registers.get(b).unwrap();
                self.registers.insert(*dest, a - b);
            }
            Inst::SubImm { a, imm, dest } => {
                let a_val = *self.registers.get(&a.expect_phys()).unwrap();
                let result = a_val - Into::<u16>::into(*imm) as u64;
                self.registers.insert(dest.expect_phys(), result);
            }

            i => self.write_output(format!("not implemented {:?}", i)),
        }

        if !is_branch {
            self.current_inst += 1;
        }

        if let Some(line) = self.inst_text.get_mut(self.current_inst) {
            line.style = Style::default().fg(ratatui::style::Color::Green).bold();
        }
    }
}

fn display_inst(
    inst: &Inst,
    inst_offset: usize,
    strings: &[(usize, String)],
    str_offsets: &HashMap<StrId, usize>,
) -> String {
    match inst {
        Inst::Add { a, b, dest } => format!("add {}, {}, {}", dest, a, b),
        Inst::AddImm { a, imm, dest } => match imm {
            AddImmVal::Fixed(imm) => format!(
                "add {}, {}, #{:#x}",
                dest.expect_phys(),
                a.expect_phys(),
                imm
            ),
            AddImmVal::String(offset) => {
                let mut string = strings
                    .iter()
                    .find_map(|(i, s)| if offset == i { Some(s.clone()) } else { None })
                    .unwrap_or_else(|| panic!("unknown string offset {}", offset));

                string.truncate(10);

                format!(
                    "add {}, #{:#x} ; \"{}\"",
                    dest.expect_phys(),
                    offset,
                    // str_offsets.get(offset).unwrap(),
                    string
                )
            }
            AddImmVal::Bss(offset) => format!(
                "add {}, {}, #{:#x} ; bss",
                dest.expect_phys(),
                a.expect_phys(),
                offset
            ),
        },
        Inst::And { a, b, dest } => {
            format!("and {}, {}, {}", dest, a, b)
        }
        Inst::Adrp { page_addr, dest } => match page_addr {
            PageAddr::Fixed(i21) => todo!(),
            PageAddr::String(_) => {
                format!("adrp {}, ? ; string table", dest)
            }
            PageAddr::Bss(offset) => format!("adrp {}, {} ; bss", dest, offset),
        },
        Inst::Branch { offset } => match offset {
            BranchOffset::Fixed(offset) => format!("b {}", fmt_offset(inst_offset, *offset)),
            // BranchOffset::Fixed(offset) => format!("b 0x{:08x}", inst_offset as i32 + *offset * 4),
            BranchOffset::Dynamic(label) => panic!(),
        },
        Inst::BranchCond { offset, cond } => todo!(),
        Inst::BranchLink { offset } => match offset {
            FnOffset::Fixed(offset) => format!("bl {}", fmt_offset(inst_offset, *offset)),
            FnOffset::Dynamic(name) => format!("bl \"{}\"", name),
        },
        Inst::BranchNotZero { offset, reg } => todo!(),
        Inst::BranchZero { offset, reg } => match offset {
            BranchOffset::Fixed(offset) => {
                format!("cbz {}, {}", reg, fmt_offset(inst_offset, *offset))
            }
            BranchOffset::Dynamic(label) => panic!(),
        },
        Inst::Cmp { a, b } => format!("cmp {}, {}", a, b),
        Inst::CmpImm { val, imm } => format!("cmp {}, #{:#x}", val, imm),
        Inst::Csel {
            cond,
            if_true,
            if_false,
            dest,
        } => format!(
            "csel {}, {}, {}, {}",
            dest,
            if_true,
            if_false,
            fmt_cond(*cond)
        ),
        Inst::Cset { inv_cond, dest } => {
            format!("cset {}, {}", dest, fmt_cond(inv_cond.inverted()))
        }
        Inst::Div { a, b, dest, signed } => {
            if *signed {
                format!("sdiv {}, {}, {}", dest, a, b)
            } else {
                format!("udiv {}, {}, {}", dest, a, b)
            }
        }
        Inst::Load {
            base,
            offset,
            dest,
            size,
        } => match *size {
            VregSize::Byte => format!(
                "ldrb {}, [{}, #0x{}]",
                dest.as_str_32bit(),
                base.expect_phys(),
                offset.fmt_size(*size)
            ),
            VregSize::Halfword => format!(
                "ldrh {}, [{}, #0x{}]",
                dest.as_str_32bit(),
                base.expect_phys(),
                offset.fmt_size(*size)
            ),
            VregSize::Word | VregSize::Doubleword => {
                format!(
                    "ldr {}, [{}, #0x{}]",
                    dest,
                    base.expect_phys(),
                    offset.fmt_size(*size)
                )
            }
            _ => todo!(),
        },
        Inst::LoadPair {
            base,
            first,
            second,
            offset,
        } => format!(
            "ldp {}, {}, [{}], #{:#x}",
            first,
            second,
            base,
            Into::<i8>::into(*offset) * 8
        ),
        Inst::Mov { src, dest } => format!("mov {}, {}", dest.expect_phys(), src.expect_phys()),
        Inst::Movk { shift, value, dest } => {
            let mut s = format!("movk {}, #{:#x}", dest, value);
            match shift {
                ImmShift16::L0 => {}
                ImmShift16::L16 => s.push_str(", LSL 16"),
                ImmShift16::L32 => s.push_str(", LSL 32"),
                ImmShift16::L48 => s.push_str(", LSL 48"),
            }
            s
        }
        Inst::Movz { shift, value, dest } => {
            let mut s = format!("movz {}, #{:#x}", dest, value);
            match shift {
                ImmShift16::L0 => {}
                ImmShift16::L16 => s.push_str(", LSL 16"),
                ImmShift16::L32 => s.push_str(", LSL 32"),
                ImmShift16::L48 => s.push_str(", LSL 48"),
            }
            s
        }
        Inst::Mul { a, b, dest } => format!("mul {}, {}, {}", dest, a, b),
        Inst::Neg { val, dest } => format!("neg {}, {}", dest, val),
        Inst::Ret { value: _ } => String::from("ret"),
        Inst::Store {
            src: source,
            base,
            offset,
            size,
        } => match *size {
            VregSize::Byte => format!(
                "strb {}, [{}, #0x{}]",
                source.as_str_32bit(),
                base.expect_phys(),
                offset.fmt_size(*size)
            ),
            VregSize::Halfword => format!(
                "strh {}, [{}, #0x{}]",
                source.as_str_32bit(),
                base.expect_phys(),
                offset.fmt_size(*size)
            ),
            VregSize::Word | VregSize::Doubleword => {
                format!(
                    "str {}, [{}, #0x{}]",
                    source,
                    base.expect_phys(),
                    offset.fmt_size(*size)
                )
            }
            _ => todo!(),
        },
        Inst::StorePair {
            base,
            first,
            second,
            offset,
        } => {
            let offset: i8 = (*offset).into();
            let sign = if offset < 0 { "-" } else { "" };
            format!(
                "stp {}, {}, [{}, #{}0x{:02x}]",
                first,
                second,
                base,
                sign,
                offset.abs() * 8
            )
        }
        Inst::Sub { a, b, dest } => format!("sub {}, {}, {}", dest, a, b),
        Inst::SubImm { a, imm, dest } => {
            format!(
                "sub {}, {}, #{:#02x}",
                dest.expect_phys(),
                a.expect_phys(),
                imm
            )
        }
        Inst::Svc { imm } => format!("svc #{:#x}", imm),
        Inst::Syscall => String::from("svc #0x80"),

        Inst::Placeholder(terminator) => todo!(),
        Inst::BeginFnCall {
            reserved_stack_size,
        } => todo!(),
        Inst::EndFnCall => todo!(),
        // inst => panic!("forgot to implement instruction {:?}", inst),
    }
}

fn fmt_cond(cond: Condition) -> &'static str {
    match cond {
        Condition::Equal => "eq",
        Condition::NotEqual => "ne",
        Condition::UnsignedGreaterOrEqual => "hs",
        Condition::UnsignedLess => "lo",
        Condition::UnsignedGreater => "hi",
        Condition::UnsignedLessOrEqual => "ls",
        Condition::SignedGreaterOrEqual => "ge",
        Condition::SignedLess => "lt",
        Condition::SignedGreater => "gt",
        Condition::SignedLessOrEqual => "le",
        Condition::Negative => "mi",
        Condition::PositiveOrZero => "pl",
        Condition::Overflow => "vs",
        Condition::NoOverflow => "vc",
        Condition::Always => "al",
        Condition::Never => "nv",
    }
}

fn fmt_offset(base: usize, offset: i32) -> String {
    format!("{:#08x}", base as i32 + offset * 4)
}
