use crate::apu2a03::APU2A03;
use crate::cartridge::{Cartridge, CartridgeError};
use crate::common::{
    interconnection::*,
    save_state::{Savable, SaveError},
    Bus, Device, MirroringProvider,
};
use crate::controller::Controller;
use crate::cpu6502::{CPUBusTrait, CPURunState, CPU6502};
use crate::display::TV;
use crate::ppu2c02::{Palette, VRam, PPU2C02};
use crate::NESKey;
use std::cell::Cell;
use std::cell::RefCell;
use std::io::Read;
use std::path::Path;
use std::rc::Rc;

struct PPUBus {
    cartridge: Rc<RefCell<dyn Bus>>,
    vram: VRam,
    palettes: Palette,
}

impl PPUBus {
    pub fn new<S>(cartridge: Rc<RefCell<S>>) -> Self
    where
        S: Bus + MirroringProvider + 'static,
    {
        PPUBus {
            cartridge: cartridge.clone(),
            vram: VRam::new(cartridge),
            palettes: Palette::new(),
        }
    }
}

impl Bus for PPUBus {
    fn read(&self, address: u16, device: Device) -> u8 {
        match address {
            0x0000..=0x1FFF => self.cartridge.borrow().read(address, device),
            0x2000..=0x3EFF => self.vram.read(address & 0x2FFF, device),
            0x3F00..=0x3FFF => self.palettes.read(address, device),
            // mirror
            0x4000..=0xFFFF => self.read(address & 0x3FFF, device),
        }
    }
    fn write(&mut self, address: u16, data: u8, device: Device) {
        match address {
            0x0000..=0x1FFF => self.cartridge.borrow_mut().write(address, data, device),
            0x2000..=0x3EFF => self.vram.write(address & 0x2FFF, data, device),
            0x3F00..=0x3FFF => self.palettes.write(address, data, device),
            // mirror
            0x4000..=0xFFFF => self.write(address & 0x3FFF, data, device),
        }
    }
}

impl Savable for PPUBus {
    fn save<W: std::io::Write>(&self, writer: &mut W) -> Result<(), SaveError> {
        self.vram.save(writer)?;
        self.palettes.save(writer)?;

        Ok(())
    }

    fn load<R: std::io::Read>(&mut self, reader: &mut R) -> Result<(), SaveError> {
        self.vram.load(reader)?;
        self.palettes.load(reader)?;

        Ok(())
    }
}

struct CPUBus {
    ram: [u8; 0x800],
    cartridge: Rc<RefCell<Cartridge>>,
    ppu: PPU2C02<PPUBus>,
    apu: APU2A03,
    contoller: Controller,
    irq_pin_change_requested: Cell<bool>,
}

impl CPUBus {
    pub fn new(
        cartridge: Rc<RefCell<Cartridge>>,
        ppu: PPU2C02<PPUBus>,
        apu: APU2A03,
        contoller: Controller,
    ) -> Self {
        CPUBus {
            cartridge,
            ram: [0; 0x800],
            ppu,
            apu,
            contoller,
            irq_pin_change_requested: Cell::new(false),
        }
    }

    fn contoller_mut(&mut self) -> &mut Controller {
        &mut self.contoller
    }
}

impl CPUBusTrait for CPUBus {
    fn read(&self, address: u16) -> u8 {
        match address {
            0x0000..=0x1FFF => self.ram[(address & 0x7FF) as usize],
            0x2000..=0x3FFF => self.ppu.read(0x2000 | (address & 0x7), Device::Cpu),
            0x4000..=0x4013 => self.apu.read(address, Device::Cpu),
            0x4014 => self.ppu.read(address, Device::Cpu),
            0x4015 => self.apu.read(address, Device::Cpu),
            0x4016 => self.contoller.read(address, Device::Cpu),
            0x4017 => self.apu.read(address, Device::Cpu),
            0x4018..=0x401F => {
                // unused CPU test mode registers
                0
            }
            0x4020..=0xFFFF => self.cartridge.borrow().read(address, Device::Cpu),
        }
    }

    fn write(&mut self, address: u16, data: u8) {
        match address {
            0x0000..=0x1FFF => self.ram[(address & 0x7FF) as usize] = data,
            0x2000..=0x3FFF => self.ppu.write(0x2000 | (address & 0x7), data, Device::Cpu),
            0x4000..=0x4013 => self.apu.write(address, data, Device::Cpu),
            0x4014 => self.ppu.write(address, data, Device::Cpu),
            0x4015 => self.apu.write(address, data, Device::Cpu),
            0x4016 => self.contoller.write(address, data, Device::Cpu),
            0x4017 => self.apu.write(address, data, Device::Cpu),
            0x4018..=0x401F => {
                // unused CPU test mode registers
            }
            0x4020..=0xFFFF => self
                .cartridge
                .borrow_mut()
                .write(address, data, Device::Cpu),
        }
    }

    fn reset(&mut self) {
        self.ram = [0; 0x800];
    }
}

impl Savable for CPUBus {
    fn save<W: std::io::Write>(&self, writer: &mut W) -> Result<(), SaveError> {
        writer.write_all(&self.ram)?;

        Ok(())
    }

    fn load<R: Read>(&mut self, reader: &mut R) -> Result<(), SaveError> {
        reader.read_exact(&mut self.ram)?;

        Ok(())
    }
}

impl PPUCPUConnection for CPUBus {
    fn is_nmi_pin_set(&self) -> bool {
        self.ppu.is_nmi_pin_set()
    }

    fn clear_nmi_pin(&mut self) {
        self.ppu.clear_nmi_pin()
    }

    fn is_dma_request(&self) -> bool {
        self.ppu.is_dma_request()
    }

    fn clear_dma_request(&mut self) {
        self.ppu.clear_dma_request()
    }

    fn dma_address(&mut self) -> u8 {
        self.ppu.dma_address()
    }

    fn send_oam_data(&mut self, address: u8, data: u8) {
        self.ppu.send_oam_data(address, data)
    }
}

impl APUCPUConnection for CPUBus {
    fn request_dmc_reader_read(&self) -> Option<u16> {
        self.apu.request_dmc_reader_read()
    }

    fn submit_dmc_buffer_byte(&mut self, byte: u8) {
        self.apu.submit_dmc_buffer_byte(byte)
    }
}

impl CPUIrqProvider for CPUBus {
    fn is_irq_change_requested(&self) -> bool {
        let result =
            self.apu.is_irq_change_requested() || self.cartridge.borrow().is_irq_change_requested();
        self.irq_pin_change_requested.set(result);
        result
    }

    fn irq_pin_state(&self) -> bool {
        if self.irq_pin_change_requested.get() {
            let mut result = self.apu.irq_pin_state();
            if self.cartridge.borrow().is_irq_change_requested() {
                result = result || self.cartridge.borrow().irq_pin_state();
            }
            result
        } else {
            false
        }
    }

    fn clear_irq_request_pin(&mut self) {
        *self.irq_pin_change_requested.get_mut() = false;
        self.cartridge.borrow_mut().clear_irq_request_pin();
        self.apu.clear_irq_request_pin();
    }
}

/// The main `NES` emulator struct, containing all components and what is actually doing the emulation.
///
/// # Example
/// ```no_run
/// use plastic_core::NES;
/// # fn display(_: &[u8]) {}
/// # fn play_audio(_: &[f32]) {}
///
/// fn main() {
///    let mut nes = NES::new("path/to/rom-file.nes").unwrap();
///    
///    loop {
///        nes.clock_for_frame();
///
///        let pixel_buffer = nes.pixel_buffer();
///        display(&pixel_buffer);
///
///        let audio_buffer = nes.audio_buffer();
///        play_audio(&audio_buffer);
///    }
/// }
/// ```
pub struct NES {
    /// The cartridge containing the ROM/CHR data
    cartridge: Rc<RefCell<Cartridge>>,

    /// CPU and containing all components through the `CPUBus`.
    cpu: CPU6502<CPUBus>,

    frame_counter: f32,
}

impl NES {
    /// Creates a new NES instance from a given file path.
    pub fn new<P: AsRef<Path>>(filename: P) -> Result<Self, CartridgeError> {
        let cartridge = Cartridge::from_file(filename)?;
        Ok(Self::create_nes(cartridge))
    }

    /// Creates a new NES instance without loading a cartridge from a file.
    ///
    /// Returns a new NES instance with an empty cartridge.
    ///
    /// Do note that running [`NES::clock_for_frame`] or [`NES::clock`] will not do anything if the cartridge is empty.
    pub fn new_without_file() -> Self {
        let cartridge = Cartridge::new_without_file();
        Self::create_nes(cartridge)
    }

    fn create_nes(cartridge: Cartridge) -> Self {
        let cartridge = Rc::new(RefCell::new(cartridge));
        let ppubus = PPUBus::new(cartridge.clone());

        let tv = TV::new();

        let ppu = PPU2C02::new(ppubus, tv);

        let apu = APU2A03::new();

        let ctrl = Controller::new();

        let cpubus = CPUBus::new(cartridge.clone(), ppu, apu, ctrl);

        let mut cpu = CPU6502::new(cpubus);

        cpu.reset();

        Self {
            cartridge,
            cpu,
            frame_counter: 0.,
        }
    }

    /// Reset the NES emulator using the same cartridge loaded already.
    pub fn reset(&mut self) {
        self.cpu.reset();
        self.cpu.reset_bus();

        let ppubus = PPUBus::new(self.cartridge.clone());

        self.cpu.bus_mut().ppu.reset(ppubus);

        self.cpu.bus_mut().apu = APU2A03::new();
    }

    /// Run the NES emulator for one video frame, which is equal to `29780` CPU cycles.
    ///
    /// This is the main function to run the emulator, call this once, and then render and play audio.
    pub fn clock_for_frame(&mut self) {
        if self.cartridge.borrow().is_empty() {
            return;
        }

        const CPU_CYCLES_PER_FRAME: f32 = 29780.5; // number of CPU cycles per loop, one full frame

        self.frame_counter += CPU_CYCLES_PER_FRAME;

        while self.frame_counter >= 0. {
            self.frame_counter -= 1.;
            self.cpu.run_next();
            self.cpu.bus_mut().apu.clock();
            {
                let ppu = &mut self.cpu.bus_mut().ppu;
                ppu.clock();
                ppu.clock();
                ppu.clock();
            }
        }
    }

    /// Run the NES emulator for one CPU cycle.
    ///
    /// This is useful for debugging and testing purposes.
    pub fn clock(&mut self) -> Option<CPURunState> {
        if self.cartridge.borrow().is_empty() {
            return None;
        }

        self.cpu.bus_mut().apu.clock();

        let r = self.cpu.run_next();
        {
            let ppu = &mut self.cpu.bus_mut().ppu;
            ppu.clock();
            ppu.clock();
            ppu.clock();
        }

        Some(r)
    }

    /// Return the pixel buffer as RGB format
    ///
    /// The size of the buffer will be [`TV_BUFFER_SIZE`][crate::nes_display::TV_BUFFER_SIZE]
    pub fn pixel_buffer(&self) -> &[u8] {
        self.cpu.bus().ppu.tv().display_pixel_buffer()
    }

    /// Take and return the audio buffer as f32 format stereo (2 channels)
    ///
    /// **Take** here means that if you call the function again, it will return an empty buffer
    /// until the emulator runs again.
    ///
    /// The emulator keeps accumulating audio samples until this function is called,
    /// so its better to call this function even if audio isn't needed in order to free up space.
    pub fn audio_buffer(&mut self) -> Vec<f32> {
        self.cpu.bus_mut().apu.take_audio_buffer()
    }

    /// Check if there is no cartridge loaded in the emulator.
    pub fn is_empty(&self) -> bool {
        self.cartridge.borrow().is_empty()
    }

    /// Set the state of a controller key. `pressed` or `released`.
    pub fn set_controller_state(&mut self, key: NESKey, pressed: bool) {
        self.cpu
            .bus_mut()
            .contoller_mut()
            .set_controller_state(key, pressed);
    }

    /// Get the name of the save state file that can be associated with the current cartridge.
    ///
    /// This is just a helper function, and the emulator implementation at [`save_state`] doesn't use it.
    ///
    /// Just a convenience.
    pub fn save_state_file_name(&self, slot: u8) -> Option<String> {
        if self.cartridge.borrow().is_empty() {
            return None;
        }

        let cart = self.cartridge.borrow();
        let cartridge_path = cart.cartridge_path();

        Some(format!(
            "{}_{}.pst",
            cartridge_path.file_stem().unwrap().to_string_lossy(),
            slot
        ))
    }

    /// Save the current state of the emulator to a writer.
    pub fn save_state<W: std::io::Write>(&self, mut writer: W) -> Result<(), SaveError> {
        self.cartridge.borrow().save(&mut writer)?;
        self.cpu.save(&mut writer)?;
        self.cpu.bus().ppu.save(&mut writer)?;
        self.cpu.bus().apu.save(&mut writer)?;

        Ok(())
    }

    /// Load the state of the emulator from a reader.
    pub fn load_state<R: std::io::Read>(&mut self, mut reader: R) -> Result<(), SaveError> {
        self.cartridge.borrow_mut().load(&mut reader)?;
        self.cpu.load(&mut reader)?;
        self.cpu.bus_mut().ppu.load(&mut reader)?;
        self.cpu.bus_mut().apu.load(&mut reader)?;

        let mut rest = Vec::new();
        reader.read_to_end(&mut rest)?;

        if !rest.is_empty() {
            return Err(SaveError::ContainExtraData);
        }

        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn cpu_bus(&self) -> &impl CPUBusTrait {
        self.cpu.bus()
    }

    #[cfg(test)]
    pub(crate) fn ppu_bus(&self) -> &impl Bus {
        self.cpu.bus().ppu.ppu_bus()
    }
}
