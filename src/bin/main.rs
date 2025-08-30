#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]

use alloc::borrow::ToOwned;
use critical_section::Mutex;
use embassy_executor::Spawner;
use embassy_net::DhcpConfig;
use embassy_net::{Config, Runner, Stack, StackResources};
use embassy_time::{Duration, Timer};
use esp_backtrace as _;
use esp_hal::clock::CpuClock;
use esp_hal::rtc_cntl::Rtc;
use esp_hal::timer::timg::TimerGroup;
use esp_wifi::wifi::WifiDevice;
use esp_wifi::{
    //    ble::controller::BleConnector,
    wifi::{ClientConfiguration, Configuration, WifiController, WifiEvent, WifiMode},
};
// use trouble_host::prelude::ExternalController;
use log::info;
use rand_core::RngCore;
use reqwless::X509;
use static_cell::StaticCell;

extern crate alloc;

const SSID: &str = "NETGEAR13";
const PASSWORD: &str = "royalphoenix978";

// This creates a default app-descriptor required by the esp-idf bootloader.
// For more information see: <https://docs.espressif.com/projects/esp-idf/en/stable/esp32/api-reference/system/app_image_format.html#application-description>
esp_bootloader_esp_idf::esp_app_desc!();

static WIFI_INIT: StaticCell<esp_wifi::EspWifiController<'static>> = StaticCell::new();
static NET_RESOURCES: StaticCell<StackResources<4>> = StaticCell::new();
static NET_STACK: StaticCell<Stack<'static>> = StaticCell::new();
static NET_RUNNER: StaticCell<Runner<'static, WifiDevice>> = StaticCell::new();
static MBEDTLS: StaticCell<esp_mbedtls::Tls<'static>> = StaticCell::new();
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
        #[unsafe(link_section = ".dram2_uninit")] size: 96 * 1024 + 463
    );

    let timer0 = TimerGroup::new(peripherals.TIMG1);
    esp_hal_embassy::init(timer0.timer0);
    let tls = MBEDTLS.uninit().write(
        esp_mbedtls::Tls::new(peripherals.SHA)
            .unwrap()
            .with_hardware_rsa(peripherals.RSA),
    );

    let rtc = &*RTC.uninit().write(Mutex::new(Rtc::new(peripherals.LPWR)));

    info!("Embassy initialized!");
    let mut rng = esp_hal::rng::Rng::new(peripherals.RNG);
    let timer1 = TimerGroup::new(peripherals.TIMG0);
    let wifi_init = WIFI_INIT.init(
        esp_wifi::init(timer1.timer0, rng.clone())
            .expect("Failed to initialize WIFI/BLE controller"),
    );

    // // find more examples https://github.com/embassy-rs/trouble/tree/main/examples/esp32
    //  let connector = BleConnector::new(wifi_init, peripherals.BT);
    // let _controller: ExternalController<_, 20> = ExternalController::new(connector);
    let (mut wifi_controller, wifi_interface) = esp_wifi::wifi::new(wifi_init, peripherals.WIFI)
        .expect("Failed to initialize WIFI controller");

    wifi_controller
        .set_mode(WifiMode::Sta)
        .expect("Failed to set wifi mode");
    spawner.spawn(wifi_connect(wifi_controller)).unwrap();

    // Setup embassy-net stack
    let config = Config::dhcpv4(DhcpConfig::default());
    let resources = NET_RESOURCES.init(StackResources::<4>::new());
    let (stack, runner) = embassy_net::new(wifi_interface.sta, config, resources, rng.next_u64());
    let stack = NET_STACK.init(stack).to_owned();
    let runner = NET_RUNNER.init(runner);
    // Spawn the network stack background task
    spawner.spawn(net_task(runner)).unwrap();

    // Wait for DHCP to assign an IP
    stack.wait_config_up().await;
    if let Some(ip_config) = stack.config_v4() {
        info!("DHCP assigned IP: {:?}", ip_config.address);
    } else {
        info!("No IP address assigned.");
    }

    // setting time correct so that tls works.
    esp_test::ntp::set_time_using_ntp(rtc, stack).await.unwrap();

    // HTTP GET to https://ifconfig.me/ip
    spawner
        .spawn(print_public_ip(tls.reference(), stack))
        .unwrap();
    loop {
        Timer::after(Duration::from_secs(5)).await;
        info!("Still alive! {:?}", esp_alloc::HEAP.stats());
    }
}

#[embassy_executor::task]
async fn net_task(runner: &'static mut Runner<'static, WifiDevice<'static>>) {
    runner.run().await;
}

#[embassy_executor::task]
async fn wifi_connect(mut controller: WifiController<'static>) {
    info!("Start connection task");
    info!("Device capabilities: {:?}", controller.capabilities());
    loop {
        if esp_wifi::wifi::wifi_state() == esp_wifi::wifi::WifiState::StaConnected {
            // wait until we're no longer connected
            controller.wait_for_event(WifiEvent::StaDisconnected).await;
            info!("Disconnected from wifi");
            Timer::after(Duration::from_millis(5000)).await
        }

        if !matches!(controller.is_started(), Ok(true)) {
            let client_config = Configuration::Client(ClientConfiguration {
                ssid: SSID.into(),
                password: PASSWORD.into(),
                ..Default::default()
            });
            controller.set_configuration(&client_config).unwrap();
            info!("Starting wifi...");
            controller.start_async().await.unwrap();
            info!("Wifi started!");
        }
        info!("Connecting to wifi...");
        match controller.connect_async().await {
            Ok(_) => info!("Wifi connected!"),
            Err(e) => {
                info!("Failed to connect to wifi: {:?}", e);
                Timer::after(Duration::from_millis(5000)).await
            }
        }
    }
}

#[embassy_executor::task(pool_size = 1)]
async fn print_public_ip(tls_reference: esp_mbedtls::TlsReference<'static>, stack: Stack<'static>) {
    // Wait until network is ready (DHCP complete)
    use embassy_net::dns::DnsSocket;
    use embassy_net::tcp::client::{TcpClient, TcpClientState};
    use embassy_time::Timer;
    use esp_mbedtls::Certificates;
    use reqwless::client::{HttpClient, TlsConfig};
    use reqwless::request::Method;

    stack.wait_config_up().await;
    info!("Network ready!");
    // Create DNS resolver
    let mut dns = DnsSocket::new(stack);

    let mut certificates = Certificates::new();
    certificates.ca_chain = Some(X509::pem(esp_test::lets_encrypt_ca::ISRG_ROOT_X1).unwrap());
    // TLS config (system defaults; you can load root certs if needed)
    let tls_config = TlsConfig::new(reqwless::TlsVersion::Tls1_2, certificates, tls_reference);

    let state = TcpClientState::<1, 1024, 1024>::new();
    let tcp_client = TcpClient::new(stack.clone(), &state);
    // Create HTTP client (with DNS + TLS)
    let mut client = HttpClient::new_with_tls(&tcp_client, &mut dns, tls_config);

    let mut res_buf = [0u8; 1024];

    loop {
        // Build request
        let mut req = client
            .request(Method::GET, "https://ifconfig.me/ip")
            .await
            .unwrap();

        let res = req.send(res_buf.as_mut_slice()).await.unwrap();
        let body = res.body().read_to_end().await.unwrap();
        info!("Public IP: {:?}", core::str::from_utf8(&body));

        Timer::after_secs(5).await
    }
}
