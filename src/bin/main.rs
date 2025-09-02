#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]

extern crate alloc;
use critical_section::Mutex;
use embassy_executor::Spawner;
use embassy_time::{Duration, Timer};
use esp_backtrace as _;
use esp_hal::clock::CpuClock;
use esp_hal::rtc_cntl::Rtc;
use esp_hal::timer::timg::TimerGroup;

// use trouble_host::prelude::ExternalController;
use log::info;
use static_cell::StaticCell;

// This creates a default app-descriptor required by the esp-idf bootloader.
// For more information see: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/app_image_format.html#application-description>
esp_bootloader_esp_idf::esp_app_desc!();

static RTC: StaticCell<Mutex<Rtc<'static>>> = StaticCell::new();

#[esp_hal_embassy::main]
async fn main(spawner: Spawner) {
    // generator version: 0.5.0

    esp_println::logger::init_logger_from_env();

    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    // Total dram_seg (available) = 192K (- 64K = 128K if bluetooth enabled)
    // Apart from bluetooth, some other big data that goes in dram_seg are
    // 1. Stack = 20K (defined in memory.x itself)
    // 2. Embassy Arena = 40K (configured in cargo.toml)
    // Total available to use if BT off = (192 - 20 - 40) = 130K (max)
    //     I am here ignoring other const and buffer we use.
    // Total available to use if BT off = (128 - 20 - 40) = 78K (max)
    esp_alloc::heap_allocator!(size: 100 * 1024);
    // COEX needs more RAM - so we've added some more
    esp_alloc::heap_allocator!(
        // Total dram2_seg in ESP32 = 98767
        //noinspection RsInvalidMacroCall
        #[unsafe(link_section = ".dram2_uninit")] size: 96 * 1024 + 463
    );

    let timer0 = TimerGroup::new(peripherals.TIMG1);
    esp_hal_embassy::init(timer0.timer0);

    let rtc = &*RTC.uninit().write(Mutex::new(Rtc::new(peripherals.LPWR)));
    let rng = esp_hal::rng::Rng::new(peripherals.RNG);

    // the wifi_interface needs to be available still end of program.
    // even though we are not using AP mode, dropping the wifi_interface.ap causing the
    // Wi-Fi to stop working.
    let wifi_interface =
        esp_test::wifi::init_wifi(spawner, rng.clone(), peripherals.WIFI, peripherals.TIMG0)
            .await
            .unwrap();
    let stack = esp_test::wifi::init_stack(spawner, wifi_interface.sta, rng)
        .await
        .unwrap();

    // Wait for DHCP to assign an IP
    stack.wait_config_up().await;
    if let Some(ip_config) = stack.config_v4() {
        info!("DHCP assigned IP: {:?}", ip_config.address);
    } else {
        info!("No IP address assigned.");
    }

    // setting time correct so that tls works.
    esp_test::net::ntp::set_real_time_using_ntp(rtc, stack)
        .await
        .unwrap();

    let net_client_factory = esp_test::net::NetClientFactory::<'_, 1, 1024, 1024>::new(
        stack,
        peripherals.SHA,
        peripherals.RSA,
    );
    let tcp_client = net_client_factory.new_tcp_client();
    let mut https_client = net_client_factory.new_https_client(&tcp_client);

    // HTTP GET to https://ifconfig.me/ip
    let mut res_buf = [0u8; 1024];
    loop {
        Timer::after(Duration::from_secs(5)).await;
        let mut req = https_client
            .request(reqwless::request::Method::GET, "https://ifconfig.me/ip")
            .await
            .unwrap();
        let res = req.send(res_buf.as_mut_slice()).await.unwrap();
        let body = res.body().read_to_end().await.unwrap();

        info!("Public IP: {:?}", core::str::from_utf8(&body));
        let used = esp_alloc::HEAP.used();
        let free = esp_alloc::HEAP.free();
        info!("Heap {}/{} used.", used, free + used);
    }
}
