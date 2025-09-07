extern crate alloc;

pub mod ca_certs;
pub mod ntp;

use embassy_net::Stack;
use embassy_net::dns::DnsSocket;
use embassy_net::tcp::client::{TcpClient, TcpClientState};
use esp_hal::peripherals;
use esp_mbedtls::{Certificates, Tls};
use reqwless::X509;
use reqwless::client::{HttpClient, TlsConfig};

pub struct NetClientFactory<'a, const N: usize, const TX_SZ: usize, const RX_SZ: usize> {
    stack: Stack<'a>,
    state: TcpClientState<N, TX_SZ, RX_SZ>,
    dns: DnsSocket<'a>,
    tls: Tls<'a>,
}

impl<'a, const N: usize, const TX_SZ: usize, const RX_SZ: usize>
    NetClientFactory<'a, N, TX_SZ, RX_SZ>
{
    pub fn new(stack: Stack<'a>, sha: peripherals::SHA<'a>, rsa: peripherals::RSA<'a>) -> Self {
        Self {
            stack,
            state: TcpClientState::new(),
            dns: DnsSocket::new(stack),
            tls: Tls::new(sha).unwrap().with_hardware_rsa(rsa),
        }
    }

    pub fn new_tcp_client(&'a self) -> TcpClient<'a, N, TX_SZ, RX_SZ> {
        TcpClient::new(self.stack, &self.state)
    }

    pub fn new_https_client(
        &'a self,
        tcp_client: &'a TcpClient<'a, N, TX_SZ, RX_SZ>,
    ) -> HttpClient<'a, TcpClient<'a, N, TX_SZ, RX_SZ>, DnsSocket<'a>> {
        let mut certificates = Certificates::new();
        let cert = &crate::config::CONFIG.net.https.ca_cert.pem;
        certificates.ca_chain =
            Some(X509::pem(cert).expect("Bug in CA certificate of lets encrypt. Failed to parse."));

        HttpClient::new_with_tls(
            tcp_client,
            &self.dns,
            TlsConfig::new(
                reqwless::TlsVersion::Tls1_2,
                certificates,
                self.tls.reference(),
            ),
        )
    }
}
