use core::net::{IpAddr, SocketAddr};
use embassy_net::dns::DnsQueryType;
use embassy_net::udp::{PacketMetadata, UdpSocket};
use embassy_net::Stack;
use embassy_time::Instant;
use esp_hal::rtc_cntl::Rtc;
use sntpc::{get_time, NtpContext, NtpResult, NtpTimestampGenerator};

const NTP_SERVER: &str = "pool.ntp.org";

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("Failed to Setup UDP for NTP")]
    SetupUdpFailed,
    #[error("Failed to resolve DNS")]
    DnsResolutionFailed,
    #[error("Failed to get NTP time")]
    NtpTimeFailed,
}

#[derive(Debug, Clone, Copy)]
struct Timestamp {
    time_us: u64,
}

impl Timestamp {
    fn new() -> Self {
        Self {
            time_us: Instant::now().as_micros(),
        }
    }
}

impl NtpTimestampGenerator for Timestamp {
    fn init(&mut self) {
        self.time_us = Instant::now().as_micros();
    }

    fn timestamp_sec(&self) -> u64 {
        self.time_us / 1_000_000
    }

    fn timestamp_subsec_micros(&self) -> u32 {
        (self.time_us % 1_000_000) as u32
    }
}

pub fn get_microseconds_from_ntp(ntp_result: NtpResult) -> u64 {
    // 1. Get the whole seconds and convert to microseconds
    let whole_seconds_micros = ntp_result.seconds as u64 * 1_000_000;

    // 2. Convert the fractional part to microseconds.
    // Use u64 to prevent overflow during the multiplication.
    let fraction_micros = (ntp_result.seconds_fraction as u64 * 1_000_000) >> 32;

    // 3. Add the two parts together.
    whole_seconds_micros + fraction_micros
}

pub async fn get_real_time_using_ntp(stack: Stack<'_>) -> Result<u64, Error> {
    info!("Waiting for network connection to be up.");
    // Wait for the tap interface to be up before continuing
    stack.wait_config_up().await;
    info!("Network ready!");

    // Create UDP socket
    let mut rx_meta = [PacketMetadata::EMPTY; 16];
    let mut rx_buffer = [0; 4096];
    let mut tx_meta = [PacketMetadata::EMPTY; 16];
    let mut tx_buffer = [0; 4096];

    let mut socket = UdpSocket::new(
        stack,
        &mut rx_meta,
        &mut rx_buffer,
        &mut tx_meta,
        &mut tx_buffer,
    );
    socket.bind(123).map_err(|_| Error::SetupUdpFailed)?;
    let context = NtpContext::new(Timestamp::new());

    let ntp_addrs = stack
        .dns_query(NTP_SERVER, DnsQueryType::A)
        .await
        .map_err(|e| {
            error!("Failed to resolve DNS: {e:?}");
            Error::DnsResolutionFailed
        })?;

    if ntp_addrs.is_empty() {
        error!("Failed to resolve DNS");
        return Err(Error::DnsResolutionFailed);
    }

    let addr: IpAddr = ntp_addrs[0].into();
    let result = get_time(SocketAddr::from((addr, 123)), &socket, context).await;

    match result {
        Ok(time) => {
            info!("Time: {time:?}");
            Ok(get_microseconds_from_ntp(time))
        }
        Err(e) => {
            error!("Error getting time: {e:?}");
            Err(Error::NtpTimeFailed)
        }
    }
}
