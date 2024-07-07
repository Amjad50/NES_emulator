use super::super::mapper::{Mapper, MappingResult};
use crate::common::{Device, MirroringMode};

pub struct Mapper1 {
    writing_shift_register: u8,

    /// 4bit0
    /// -----
    /// CPPMM
    /// |||||
    /// |||++- Mirroring (0: one-screen, lower bank; 1: one-screen, upper bank;
    /// |||               2: vertical; 3: horizontal)
    /// |++--- PRG ROM bank mode (0, 1: switch 32 KB at $8000, ignoring low bit of bank number;
    /// |                         2: fix first bank at $8000 and switch 16 KB bank at $C000;
    /// |                         3: fix last bank at $C000 and switch 16 KB bank at $8000)
    /// +----- CHR ROM bank mode (0: switch 8 KB at a time; 1: switch two separate 4 KB banks)
    control_register: u8,

    /// 4bit0
    /// -----
    /// CCCCC
    /// |||||
    /// +++++- Select 4 KB or 8 KB CHR bank at PPU $0000 (low bit ignored in 8 KB mode)
    ///
    /// OR
    ///
    /// 4bit0
    /// -----
    /// ExxxC
    /// |   |
    /// |   +- Select 4 KB CHR RAM bank at PPU $0000 (ignored in 8 KB mode)
    /// +----- PRG RAM disable (0: enable, 1: open bus)
    ///
    /// OR
    ///
    /// 4bit0
    /// -----
    /// PSSxC
    /// ||| |
    /// ||| +- Select 4 KB CHR RAM bank at PPU $0000 (ignored in 8 KB mode)
    /// |++--- Select 8 KB PRG RAM bank
    /// +----- Select 256 KB PRG ROM bank
    chr_0_bank: u8,

    /// 4bit0
    /// -----
    /// CCCCC
    /// |||||
    /// +++++- Select 4 KB CHR bank at PPU $1000 (ignored in 8 KB mode)
    ///
    /// OR
    ///
    /// 4bit0
    /// -----
    /// ExxxC
    /// |   |
    /// |   +- Select 4 KB CHR RAM bank at PPU $0000 (ignored in 8 KB mode)
    /// +----- PRG RAM disable (0: enable, 1: open bus) (ignored in 8 KB mode)
    ///
    /// OR
    ///
    /// 4bit0
    /// -----
    /// PSSxC
    /// ||| |
    /// ||| +- Select 4 KB CHR RAM bank at PPU $0000 (ignored in 8 KB mode)
    /// |++--- Select 8 KB PRG RAM bank (ignored in 8 KB mode)
    /// +----- Select 256 KB PRG ROM bank (ignored in 8 KB mode)
    chr_1_bank: u8,

    /// 4bit0
    /// -----
    /// -PPPP
    ///  ||||
    ///  ++++- Select 16 KB PRG ROM bank (low bit ignored in 32 KB mode)
    prg_bank: u8,

    /// 4bit0
    /// -----
    /// R----
    /// |
    /// +----- PRG RAM chip enable (0: enabled; 1: disabled; ignored on MMC1A)
    prg_ram_enable: bool,

    /// is using CHR ram
    is_chr_ram: bool,

    /// in 4kb units
    chr_count: u8,

    /// in 16kb units
    prg_count: u8,

    /// in 8kb units
    prg_ram_count: u8,
}

impl Mapper1 {
    pub fn new() -> Self {
        Self {
            writing_shift_register: 0b10000,
            control_register: 0x0C, // power-up
            chr_0_bank: 0,
            chr_1_bank: 0,
            prg_bank: 0,

            prg_ram_enable: false,

            is_chr_ram: false,

            chr_count: 0,
            prg_count: 0,

            prg_ram_count: 0,
        }
    }

    fn reset_shift_register(&mut self) {
        // the 1 is used to indicate that the shift register is full when it
        // reaches the end
        self.writing_shift_register = 0b10000;
    }

    fn get_mirroring(&self) -> u8 {
        self.control_register & 0b00011
    }

    fn get_prg_bank(&self) -> u8 {
        self.prg_bank & 0b1111
    }

    fn is_prg_32kb_mode(&self) -> bool {
        self.control_register & 0b01000 == 0
    }

    /// this should be used in combination with `is_PRG_32kb_mode`
    /// this function will assume that the mapper is in 16kb mode
    /// if the first bank is fixed at 0x8000 and the second chunk of 16kb
    /// is switchable, this should return `true`
    /// if the last bank is fixed into 0xC000 and the first chunk of 16kb
    /// is switchable, this should return `false`
    fn is_first_prg_chunk_fixed(&self) -> bool {
        self.control_register & 0b00100 == 0
    }

    fn is_chr_8kb_mode(&self) -> bool {
        self.control_register & 0b10000 == 0
    }

    fn is_prg_ram_enabled(&self) -> bool {
        // 8KB (SNROM) and not in 512KB PRG mode
        let snrom_prg_ram_enabled = if self.chr_count == 2 && self.prg_count <= 16 {
            if self.is_chr_8kb_mode() {
                self.chr_0_bank & 0x10 == 0
            } else {
                self.chr_1_bank & 0x10 == 0
            }
        } else {
            // only depend on `self.prg_ram_enable`
            true
        };

        self.prg_ram_enable && snrom_prg_ram_enabled
    }

    fn map_ppu(&self, address: u16) -> MappingResult {
        let mut bank = if self.is_chr_8kb_mode() {
            self.chr_0_bank & 0b11110
        } else if address <= 0x0FFF {
            self.chr_0_bank
        } else if (0x1000..=0x1FFF).contains(&address) {
            self.chr_1_bank
        } else {
            unreachable!()
        } as usize;

        bank %= self.chr_count as usize;

        let start_of_bank = 0x1000 * bank;

        let mask = if self.is_chr_8kb_mode() {
            0x1FFF
        } else {
            0xFFF
        };

        // add the offset
        MappingResult::Allowed(start_of_bank + (address & mask) as usize)
    }

    fn map_prg_ram(&self, address: u16) -> MappingResult {
        if self.is_prg_ram_enabled() && self.prg_ram_count > 0 {
            let bank = if self.prg_ram_count > 1 {
                if self.is_chr_8kb_mode() {
                    (self.chr_0_bank >> 2) & 0x3
                } else {
                    (self.chr_1_bank >> 2) & 0x3
                }
            } else {
                0
            } as usize;
            MappingResult::Allowed(bank * 0x2000 + (address & 0x1FFF) as usize)
        } else {
            MappingResult::Denied
        }
    }
}

impl Mapper for Mapper1 {
    fn init(&mut self, prg_count: u8, is_chr_ram: bool, chr_count: u8, sram_count: u8) {
        self.prg_count = prg_count;
        self.chr_count = chr_count * 2; // since this passed as the number of 8kb banks
        self.is_chr_ram = is_chr_ram;

        self.prg_bank = prg_count - 1; // power-up, should be all set?
        self.control_register = 0b11100; // power-up state

        self.prg_ram_count = sram_count;

        self.reset_shift_register();
    }

    fn map_read(&self, address: u16, device: Device) -> MappingResult {
        match device {
            Device::Cpu => {
                match address {
                    0x6000..=0x7FFF => self.map_prg_ram(address),
                    0x8000..=0xFFFF => {
                        let mut bank = if self.is_prg_32kb_mode() {
                            // ignore last bit
                            self.get_prg_bank() & 0b11110
                        } else if (0x8000..=0xBFFF).contains(&address) {
                            if self.is_first_prg_chunk_fixed() {
                                0
                            } else {
                                self.get_prg_bank()
                            }
                        } else if address >= 0xC000 {
                            if self.is_first_prg_chunk_fixed() {
                                self.get_prg_bank()
                            } else {
                                // last bank
                                self.prg_count - 1
                            }
                        } else {
                            unreachable!();
                        } as usize;

                        if self.prg_count > 16 && self.chr_count == 2 {
                            let prg_high_bit_512_mode = if self.is_chr_8kb_mode() {
                                self.chr_0_bank & 0x10
                            } else {
                                self.chr_1_bank & 0x10
                            } as usize;

                            bank |= prg_high_bit_512_mode;
                        }

                        bank %= self.prg_count as usize;

                        let start_of_bank = 0x4000 * bank;

                        let last_bank = 0x4000 * (self.prg_count - 1) as usize;

                        // since banks can be odd in number, we don't want to go out
                        // of bounds, but this solution does mirroring, in case of
                        // a possible out of bounds, but not sure what is the correct
                        // solution
                        let mask = if self.is_prg_32kb_mode() && start_of_bank != last_bank {
                            0x7FFF
                        } else {
                            0x3FFF
                        };

                        // add the offset
                        MappingResult::Allowed(start_of_bank + (address & mask) as usize)
                    }
                    0x4020..=0x5FFF => MappingResult::Denied,
                    _ => unreachable!(),
                }
            }
            Device::Ppu => {
                if address < 0x2000 {
                    self.map_ppu(address)
                } else {
                    unreachable!()
                }
            }
        }
    }

    fn map_write(&mut self, address: u16, data: u8, device: Device) -> MappingResult {
        match device {
            Device::Cpu => {
                match address {
                    0x6000..=0x7FFF => self.map_prg_ram(address),
                    0x8000..=0xFFFF => {
                        if data & 0x80 != 0 {
                            self.reset_shift_register();
                        } else {
                            let should_save = self.writing_shift_register & 1 != 0;
                            // shift
                            self.writing_shift_register >>= 1;
                            self.writing_shift_register |= (data & 1) << 4;

                            // reached the end, then save
                            if should_save {
                                let result = self.writing_shift_register & 0b11111;
                                match address {
                                    0x8000..=0x9FFF => self.control_register = result,
                                    0xA000..=0xBFFF => self.chr_0_bank = result,
                                    0xC000..=0xDFFF => self.chr_1_bank = result,
                                    0xE000..=0xFFFF => {
                                        self.prg_bank = result & 0xF;
                                        self.prg_ram_enable = result & 0x10 == 0;
                                    }
                                    _ => {
                                        unreachable!();
                                    }
                                }

                                self.reset_shift_register();
                            }
                        }
                        MappingResult::Denied
                    }
                    0x4020..=0x5FFF => MappingResult::Denied,
                    _ => unreachable!(),
                }
            }
            Device::Ppu => {
                // CHR RAM
                if self.is_chr_ram && address <= 0x1FFF {
                    self.map_ppu(address)
                } else {
                    MappingResult::Denied
                }
            }
        }
    }

    fn is_hardwired_mirrored(&self) -> bool {
        false
    }

    fn nametable_mirroring(&self) -> MirroringMode {
        [
            MirroringMode::SingleScreenLowBank,
            MirroringMode::SingleScreenHighBank,
            MirroringMode::Vertical,
            MirroringMode::Horizontal,
        ][self.get_mirroring() as usize]
    }

    fn save_state_size(&self) -> usize {
        10
    }

    fn save_state(&self) -> Vec<u8> {
        vec![
            self.writing_shift_register,
            self.control_register,
            self.chr_0_bank,
            self.chr_1_bank,
            self.prg_bank,
            self.chr_count,
            self.prg_count,
            self.prg_ram_count,
            self.prg_ram_enable as u8,
            self.is_chr_ram as u8,
        ]
    }

    fn load_state(&mut self, data: Vec<u8>) {
        self.writing_shift_register = data[0];
        self.control_register = data[1];
        self.chr_0_bank = data[2];
        self.chr_1_bank = data[3];
        self.prg_bank = data[4];
        self.chr_count = data[5];
        self.prg_count = data[6];
        self.prg_ram_count = data[7];
        self.prg_ram_enable = data[8] != 0;
        self.is_chr_ram = data[9] != 0;
    }
}
