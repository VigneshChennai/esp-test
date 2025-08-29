#![no_std]
#![no_main]
#![deny(
    clippy::mem_forget,
    reason = "mem::forget is generally not safe to do with esp_hal types, especially those \
    holding buffers for the duration of a data transfer."
)]

use alloc::borrow::ToOwned;
use embassy_executor::Spawner;
use embassy_net::DhcpConfig;
use embassy_net::{Config, Runner, Stack, StackResources};
use embassy_time::{Duration, Timer};
use esp_backtrace as _;
use esp_hal::clock::CpuClock;
use esp_hal::timer::timg::TimerGroup;
use esp_wifi::wifi::WifiDevice;
use esp_wifi::{
//    ble::controller::BleConnector,
    wifi::{ClientConfiguration, Configuration, WifiController, WifiEvent, WifiMode},
};
// use trouble_host::prelude::ExternalController;
use log::info;
use rand_core::RngCore;
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


#[esp_hal_embassy::main]
async fn main(spawner: Spawner) {
    // generator version: 0.5.0

    esp_println::logger::init_logger_from_env();

    let config = esp_hal::Config::default().with_cpu_clock(CpuClock::max());
    let peripherals = esp_hal::init(config);

    //esp_alloc::heap_allocator!(size: 32 * 1024);
    // COEX needs more RAM - so we've added some more
    esp_alloc::heap_allocator!(#[unsafe(link_section = ".dram2_uninit")] size: 64 * 1024);

    let timer0 = TimerGroup::new(peripherals.TIMG1);
    esp_hal_embassy::init(timer0.timer0);

    info!("Embassy initialized!");
    let mut rng = esp_hal::rng::Rng::new(peripherals.RNG); 
    let timer1 = TimerGroup::new(peripherals.TIMG0);
    let wifi_init = WIFI_INIT.init(
        esp_wifi::init(timer1.timer0, rng.clone()).expect("Failed to initialize WIFI/BLE controller"),
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

    // HTTP GET to https://ifconfig.me/ip
    spawner.spawn(print_public_ip(rng.clone(), stack)).unwrap();
    spawner.spawn(print_public_ip(rng.clone(), stack)).unwrap();
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

#[embassy_executor::task(pool_size = 2)]
async fn print_public_ip(mut rng: esp_hal::rng::Rng, stack: Stack<'static>) {
    // Wait until network is ready (DHCP complete)
    use embassy_net::dns::DnsSocket;
    use embassy_time::Timer;
    use reqwless::client::HttpClient;
    use reqwless::client::TlsConfig;
    use reqwless::client::TlsVerify;
    use reqwless::request::Method;
    use rand_core::RngCore;
    use embassy_net::tcp::client::{TcpClientState, TcpClient};

    loop {
        if stack.is_config_up() {
            break;
        }
        Timer::after_secs(1).await;
    }
    info!("Network ready!");

    // Create DNS resolver
    let mut dns = DnsSocket::new(stack);
    let mut rx = [0u8; 1024 * 16];
    let mut tx = [0u8; 1024 * 16];
    // TLS config (system defaults; you can load root certs if needed)
    let tls = TlsConfig::new(
        rng.next_u64(),
        &mut rx,
        &mut tx,
        TlsVerify::None
    );

    let state = TcpClientState::<1, 1024, 1024>::new();
    let tcp_client = TcpClient::new(stack.clone(), &state);
    // Create HTTP client (with DNS + TLS)
    let mut client = HttpClient::new_with_tls(&tcp_client, &mut dns, tls);
    
    let mut res_buf = [0u8; 1024];
    
    loop {
        // Build request
        let mut req = client.request(Method::GET, "https://ifconfig.me/ip").await.unwrap();
        
        let res = req.send(res_buf.as_mut_slice()).await.unwrap();
        let body = res.body().read_to_end().await.unwrap();
        info!("Public IP: {:?}", core::str::from_utf8(&body));

        Timer::after_secs(5).await
    }
    
}