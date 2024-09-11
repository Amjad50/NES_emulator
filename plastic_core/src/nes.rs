use crate::apu2a03::APU2A03;
use crate::cartridge::{Cartridge, CartridgeError};
use crate::common::{
    interconnection::*,
    save_state::{Savable, SaveError},
    Bus, Device, MirroringProvider,
};
use crate::controller::Controller;
use crate::cpu6502::{CPUBusTrait, CPU6502};
use crate::display::TV;
use crate::ppu2c02::{Palette, VRam, PPU2C02};
use std::cell::Cell;
use std::cell::RefCell;
use std::io::Read;
use std::path::Path;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

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
    ppu: Rc<RefCell<PPU2C02<PPUBus>>>,
    apu: Rc<RefCell<APU2A03>>,
    contoller: Controller,
    irq_pin_change_requested: Cell<bool>,
}

impl CPUBus {
    pub fn new(
        cartridge: Rc<RefCell<Cartridge>>,
        ppu: Rc<RefCell<PPU2C02<PPUBus>>>,
        apu: Rc<RefCell<APU2A03>>,
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
            0x2000..=0x3FFF => self
                .ppu
                .borrow()
                .read(0x2000 | (address & 0x7), Device::Cpu),
            0x4000..=0x4013 => self.apu.borrow().read(address, Device::Cpu),
            0x4014 => self.ppu.borrow().read(address, Device::Cpu),
            0x4015 => self.apu.borrow().read(address, Device::Cpu),
            0x4016 => self.contoller.read(address, Device::Cpu),
            0x4017 => self.apu.borrow().read(address, Device::Cpu),
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
            0x2000..=0x3FFF => {
                self.ppu
                    .borrow_mut()
                    .write(0x2000 | (address & 0x7), data, Device::Cpu)
            }
            0x4000..=0x4013 => self.apu.borrow_mut().write(address, data, Device::Cpu),
            0x4014 => self.ppu.borrow_mut().write(address, data, Device::Cpu),
            0x4015 => self.apu.borrow_mut().write(address, data, Device::Cpu),
            0x4016 => self.contoller.write(address, data, Device::Cpu),
            0x4017 => self.apu.borrow_mut().write(address, data, Device::Cpu),
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
        self.ppu.borrow().is_nmi_pin_set()
    }

    fn clear_nmi_pin(&mut self) {
        self.ppu.borrow_mut().clear_nmi_pin()
    }

    fn is_dma_request(&self) -> bool {
        self.ppu.borrow_mut().is_dma_request()
    }

    fn clear_dma_request(&mut self) {
        self.ppu.borrow_mut().clear_dma_request()
    }

    fn dma_address(&mut self) -> u8 {
        self.ppu.borrow_mut().dma_address()
    }

    fn send_oam_data(&mut self, address: u8, data: u8) {
        self.ppu.borrow_mut().send_oam_data(address, data)
    }
}

impl APUCPUConnection for CPUBus {
    fn request_dmc_reader_read(&self) -> Option<u16> {
        self.apu.borrow().request_dmc_reader_read()
    }

    fn submit_dmc_buffer_byte(&mut self, byte: u8) {
        self.apu.borrow_mut().submit_dmc_buffer_byte(byte)
    }
}

impl CPUIrqProvider for CPUBus {
    fn is_irq_change_requested(&self) -> bool {
        let result = self.apu.borrow().is_irq_change_requested()
            || self.cartridge.borrow().is_irq_change_requested();
        self.irq_pin_change_requested.set(result);
        result
    }

    fn irq_pin_state(&self) -> bool {
        if self.irq_pin_change_requested.get() {
            let mut result = self.apu.borrow().irq_pin_state();
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
        self.apu.borrow_mut().clear_irq_request_pin();
    }
}

pub struct NES {
    cartridge: Rc<RefCell<Cartridge>>,
    cpu: CPU6502<CPUBus>,
    ppu: Rc<RefCell<PPU2C02<PPUBus>>>,
    apu: Rc<RefCell<APU2A03>>,
    image: Arc<Mutex<Vec<u8>>>,
}

impl NES {
    pub fn new<P: AsRef<Path>>(filename: P) -> Result<Self, CartridgeError> {
        let cartridge = Cartridge::from_file(filename)?;

        Ok(Self::create_nes(cartridge))
    }

    pub fn new_without_file() -> Self {
        let cartridge = Cartridge::new_without_file();

        Self::create_nes(cartridge)
    }

    fn create_nes(cartridge: Cartridge) -> Self {
        let cartridge = Rc::new(RefCell::new(cartridge));
        let ppubus = PPUBus::new(cartridge.clone());

        let tv = TV::new();
        let image = tv.get_image_clone();

        let ppu = PPU2C02::new(ppubus, tv);

        let ppu = Rc::new(RefCell::new(ppu));

        let apu = Rc::new(RefCell::new(APU2A03::new()));

        let ctrl = Controller::new();

        let cpubus = CPUBus::new(cartridge.clone(), ppu.clone(), apu.clone(), ctrl);

        let mut cpu = CPU6502::new(cpubus);

        cpu.reset();

        Self {
            cartridge,
            cpu,
            ppu,
            apu,
            image,
        }
    }

    pub fn reset(&mut self) {
        self.cpu.reset();
        self.cpu.reset_bus();

        let ppubus = PPUBus::new(self.cartridge.clone());

        self.ppu.borrow_mut().reset(ppubus);

        self.apu.replace(APU2A03::new());
    }

    pub fn clock_for_frame(&mut self) {
        if self.cartridge.borrow().is_empty() {
            return;
        }

        const N: usize = 29780; // number of CPU cycles per loop, one full frame

        for _ in 0..N {
            self.apu.borrow_mut().clock();

            self.cpu.run_next();
            {
                let mut ppu = self.ppu.borrow_mut();
                ppu.clock();
                ppu.clock();
                ppu.clock();
            }
        }
    }

    /// Return the pixel buffer as RGBA format
    pub fn pixel_buffer(&self) -> Arc<Mutex<Vec<u8>>> {
        self.image.clone()
    }

    pub fn audio_buffer(&self) -> Vec<f32> {
        self.apu.borrow().take_audio_buffer()
    }

    pub fn is_empty(&self) -> bool {
        self.cartridge.borrow().is_empty()
    }

    pub fn controller(&mut self) -> &mut Controller {
        self.cpu.bus_mut().contoller_mut()
    }

    // fn get_base_save_state_folder(&self) -> Option<PathBuf> {
    //     if let Some(proj_dirs) = ProjectDirs::from("Amjad50", "Plastic", "Plastic") {
    //         let base_saved_states_dir = proj_dirs.data_local_dir().join("saved_states");
    //         // Linux:   /home/../.local/share/plastic/saved_states
    //         // Windows: C:\Users\..\AppData\Local\Plastic\Plastic\data\saved_states
    //         // macOS:   /Users/../Library/Application Support/Amjad50.Plastic.Plastic/saved_states

    //         fs::create_dir_all(&base_saved_states_dir).ok()?;

    //         Some(base_saved_states_dir)
    //     } else {
    //         None
    //     }
    // }

    // fn get_save_state_file_path(&self, slot: u8) -> Option<Box<Path>> {
    //     if self.cartridge.borrow().is_empty() {
    //         return None;
    //     }

    //     let cartridge_path = self.cartridge.borrow().cartridge_path().to_path_buf();

    //     self.get_base_save_state_folder()
    //         .map(|base_saved_states_dir| {
    //             base_saved_states_dir
    //                 .join(format!(
    //                     "{}_{}.pst",
    //                     cartridge_path.file_stem().unwrap().to_string_lossy(),
    //                     slot
    //                 ))
    //                 .into_boxed_path()
    //         })
    // }

    // fn get_present_save_states(&self) -> Option<Vec<u8>> {
    //     if self.cartridge.borrow().is_empty() {
    //         return None;
    //     }

    //     let cartridge_path = self.cartridge.borrow().cartridge_path().to_path_buf();

    //     if let Some(base_saved_states_dir) = self.get_base_save_state_folder() {
    //         let saved_states_files_regex = Regex::new(&format!(
    //             r"{}_(\d*).pst",
    //             regex::escape(&cartridge_path.file_stem().unwrap().to_string_lossy()),
    //         ))
    //         .ok()?;

    //         Some(
    //             fs::read_dir(base_saved_states_dir)
    //                 .ok()?
    //                 .filter_map(|path| {
    //                     if path.as_ref().ok()?.file_type().ok()?.is_file() {
    //                         Some(path.ok()?.path())
    //                     } else {
    //                         None
    //                     }
    //                 })
    //                 .filter_map(|path| {
    //                     saved_states_files_regex
    //                         .captures(path.file_name()?.to_str()?)?
    //                         .get(1)?
    //                         .as_str()
    //                         .parse::<u8>()
    //                         .ok()
    //                 })
    //                 .collect::<Vec<u8>>(),
    //         )
    //     } else {
    //         None
    //     }
    // }

    // pub fn save_state(&self, slot: u8) -> Result<(), SaveError> {
    //     if let Some(path) = self.get_save_state_file_path(slot) {
    //         let mut file = File::create(path)?;

    //         self.cartridge.borrow().save(&mut file)?;
    //         self.cpu.save(&mut file)?;
    //         self.ppu.borrow().save(&mut file)?;
    //         self.apu.borrow().save(&mut file)?;

    //         Ok(())
    //     } else {
    //         Err(SaveError::Others)
    //     }
    // }

    // pub fn load_state(&mut self, slot: u8) -> Result<(), SaveError> {
    //     if let Some(path) = self.get_save_state_file_path(slot) {
    //         if path.exists() {
    //             let mut file = File::open(path)?;

    //             self.cartridge.borrow_mut().load(&mut file)?;
    //             self.cpu.load(&mut file)?;
    //             self.ppu.borrow_mut().load(&mut file)?;
    //             self.apu.borrow_mut().load(&mut file)?;

    //             let mut rest = Vec::new();
    //             file.read_to_end(&mut rest)?;

    //             if !rest.is_empty() {
    //                 return Err(SaveError::Others);
    //             }

    //             if !self.paused {
    //                 self.apu.borrow().play();
    //             }

    //             Ok(())
    //         } else {
    //             Err(SaveError::IoError(std::io::Error::new(
    //                 std::io::ErrorKind::NotFound,
    //                 "save file not found",
    //             )))
    //         }
    //     } else {
    //         Err(SaveError::Others)
    //     }
    // }

    // calculate a new view based on the window size
    // pub fn run(&mut self) {
    //     let image = self.image.clone();
    //     let ctrl_state = self.ctrl_state.clone();
    //     let mut frame_limiter = FrameLimiter::new(60);

    //     let (ui_to_nes_sender, ui_to_nes_receiver) = channel::<UiEvent>();
    //     let (nes_to_ui_sender, nes_to_ui_receiver) = channel::<BackendEvent>();

    //     let mut ui = self.ui.take().unwrap();

    //     let ui_thread_handler = std::thread::spawn(move || {
    //         ui.run_ui_loop(
    //             ui_to_nes_sender.clone(),
    //             nes_to_ui_receiver,
    //             image,
    //             ctrl_state,
    //         );
    //         ui_to_nes_sender.send(UiEvent::Exit).unwrap();
    //     });

    //     self.cpu.reset();

    //     const N: usize = 29780; // number of CPU cycles per loop, one full frame

    //     // just a way to duplicate code, its not meant to be efficient way to do it
    //     // I used this, since `self` cannot be referenced here and anywhere else at
    //     // the same time.
    //     macro_rules! handle_apu_after_reset {
    //         () => {
    //             if !self.paused {
    //                 self.apu.borrow().play();
    //             }
    //         };
    //     }

    //     macro_rules! send_present_save_states_to_ui {
    //         () => {
    //             if let Some(states) = self.get_present_save_states() {
    //                 nes_to_ui_sender
    //                     .send(BackendEvent::PresentStates(states))
    //                     .unwrap();
    //             }
    //         };
    //     }

    //     // first time
    //     handle_apu_after_reset!();

    //     send_present_save_states_to_ui!();

    //     // run the emulator loop
    //     loop {
    //         // check for events
    //         if let Ok(event) = ui_to_nes_receiver.try_recv() {
    //             match event {
    //                 UiEvent::Exit => break,
    //                 UiEvent::Reset => {
    //                     self.reset();
    //                     handle_apu_after_reset!();
    //                     send_present_save_states_to_ui!();
    //                 }

    //                 UiEvent::LoadRom(file_location) => {
    //                     let cartridge = Cartridge::from_file(file_location);
    //                     if let Ok(cartridge) = cartridge {
    //                         self.cartridge.replace(cartridge);
    //                         self.reset();
    //                         handle_apu_after_reset!();
    //                     } else {
    //                         println!("This game is not supported yet");
    //                     }
    //                     send_present_save_states_to_ui!();
    //                 }
    //                 UiEvent::SaveState(slot) => {
    //                     // only if there is a game
    //                     if !self.cartridge.borrow().is_empty() {
    //                         if let Err(err) = self.save_state(slot) {
    //                             eprintln!("Error in saving the state: {}", err);
    //                         }
    //                         send_present_save_states_to_ui!();
    //                     }
    //                 }
    //                 UiEvent::LoadState(slot) => {
    //                     // only if there is a game
    //                     if !self.cartridge.borrow().is_empty() {
    //                         if let Err(err) = self.load_state(slot) {
    //                             eprintln!("Error in loading the state: {}", err);
    //                         }
    //                         send_present_save_states_to_ui!();
    //                     }
    //                 }
    //             }
    //         }

    // }
}
