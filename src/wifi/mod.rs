extern crate alloc;

use alloc::borrow::ToOwned;
use embassy_executor::Spawner;
use embassy_net::{Config, DhcpConfig, Runner, Stack, StackResources};
use embassy_time::{Duration, Timer};
use esp_hal::{peripherals, rng::Rng, timer::timg::TimerGroup};
use esp_wifi::wifi::{
    ClientConfiguration, Configuration, Interfaces, WifiController, WifiDevice, WifiEvent, WifiMode,
};
use log::info;
use rand_core::RngCore;
use static_cell::StaticCell;

static WIFI_INIT: StaticCell<esp_wifi::EspWifiController<'static>> = StaticCell::new();
static NET_RESOURCES: StaticCell<StackResources<4>> = StaticCell::new();
static NET_STACK: StaticCell<Stack<'static>> = StaticCell::new();
static NET_RUNNER: StaticCell<Runner<'static, WifiDevice>> = StaticCell::new();

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("WifiInitError")]
    WifiInitError,
    #[error("WifiModeError")]
    WifiModeError,
    #[error("WifiConnectError")]
    WifiConnectError,
    #[error("NetStackError")]
    NetStackError,
    #[error("NetRunnerError")]
    NetRunnerError,
    #[error("NetTaskError")]
    NetTaskError,
}

const SSID: &str = "NETGEAR13";
const PASSWORD: &str = "royalphoenix978";

pub async fn init_wifi(
    spawner: Spawner,
    rng: Rng,
    wifi: peripherals::WIFI<'static>,
    timg0: peripherals::TIMG0<'static>,
) -> Result<Interfaces<'static>, Error> {
    info!("Embassy initialized!");
    let timer1 = TimerGroup::new(timg0);
    let wifi_init =
        WIFI_INIT.init(esp_wifi::init(timer1.timer0, rng).map_err(|_| Error::WifiInitError)?);

    // // find more examples https://github.com/embassy-rs/trouble/tree/main/examples/esp32
    //  let connector = BleConnector::new(wifi_init, peripherals.BT);
    // let _controller: ExternalController<_, 20> = ExternalController::new(connector);
    let (mut wifi_controller, wifi_interface) =
        esp_wifi::wifi::new(wifi_init, wifi).expect("Failed to initialize WIFI controller");
    wifi_controller
        .set_mode(WifiMode::Sta)
        .expect("Failed to set wifi mode");
    spawner
        .spawn(wifi_connect(wifi_controller))
        .map_err(|_| Error::WifiConnectError)?;
    Ok(wifi_interface)
}

pub async fn init_stack<'a>(
    spawner: Spawner,
    sta: WifiDevice<'static>,
    mut rng: Rng,
) -> Result<Stack<'a>, Error> {
    // Setup embassy-net stack
    let config = Config::dhcpv4(DhcpConfig::default());
    let resources = NET_RESOURCES.init(StackResources::<4>::new());
    let (stack, runner) = embassy_net::new(sta, config, resources, rng.next_u64());
    let stack = NET_STACK.init(stack).to_owned();
    let runner = NET_RUNNER.init(runner);

    // Spawn the network stack background task
    spawner
        .spawn(net_task(runner))
        .map_err(|_| Error::NetTaskError)?;
    stack.wait_config_up().await;
    Ok(stack)
}

#[embassy_executor::task]
pub async fn net_task(runner: &'static mut Runner<'static, WifiDevice<'static>>) {
    runner.run().await;
}

#[embassy_executor::task]
pub async fn wifi_connect(mut controller: WifiController<'static>) {
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
                info!("Failed to connect to wifi: {e:?}");
                Timer::after(Duration::from_millis(5000)).await
            }
        }
    }
}
