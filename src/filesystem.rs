use embedded_storage::nor_flash::NorFlash;
use embedded_storage::ReadStorage;
use esp_storage::FlashStorage;
use littlefs2::fs::Allocation;
use littlefs2::io::Error;
use log::error;
use static_cell::StaticCell;
const FLASH_OFFSET: u32 = 0x380000;
const FLASH_SIZE: usize = 300 * 1024; // 300 KB
const BLOCK_SIZE: usize = 4096; // Flash sector size
const READ_SIZE: usize = 16; // Safe minimum read
const WRITE_SIZE: usize = 16; // Safe minimum write

pub struct AppStorage {
    flash_storage: FlashStorage,
}

impl AppStorage {
    pub fn new() -> Self {
        Self {
            flash_storage: FlashStorage::new(),
        }
    }
}
impl littlefs2::driver::Storage for AppStorage {
    const READ_SIZE: usize = READ_SIZE;
    const WRITE_SIZE: usize = WRITE_SIZE;
    const BLOCK_SIZE: usize = BLOCK_SIZE;
    const BLOCK_COUNT: usize = FLASH_SIZE / BLOCK_SIZE;

    type CACHE_SIZE = typenum::U256; // LittleFS cache size
    type LOOKAHEAD_SIZE = typenum::U1;

    fn read(&mut self, off: usize, buf: &mut [u8]) -> Result<usize, Error> {
        let addr = FLASH_OFFSET + off as u32;
        self.flash_storage
            .read(addr, buf)
            .map(|_| buf.len())
            .map_err(|e| {
                error!("Flash read error: {:?}", e);
                Error::IO
            })
    }

    fn write(&mut self, off: usize, data: &[u8]) -> Result<usize, Error> {
        let addr = FLASH_OFFSET + off as u32;
        embedded_storage::Storage::write(&mut self.flash_storage, addr, data)
            .map(|_| data.len())
            .map_err(|e| {
                error!("Flash write error: {:?}", e);
                Error::IO
            })
    }

    fn erase(&mut self, off: usize, len: usize) -> Result<usize, Error> {
        let addr = FLASH_OFFSET + off as u32;
        let end = addr + len as u32;
        self.flash_storage
            .erase(addr, end)
            .map(|_| len)
            .map_err(|e| {
                error!("Flash erase error: {:?}", e);
                Error::IO
            })
    }
}

pub static ALLOC: StaticCell<Allocation<AppStorage>> = StaticCell::new();
pub static STORAGE: StaticCell<AppStorage> = StaticCell::new();
