extern crate alloc;

use alloc::boxed::Box;
use embassy_net::Stack;
use embassy_net::dns::DnsSocket;
use embassy_net::tcp::client::{TcpClient, TcpClientState};
use esp_mbedtls::Certificates;
use reqwless::client::{HttpClient, HttpRequestHandle, TlsConfig};
use reqwless::request::Method;
use reqwless::{Error, X509};

pub struct Client<const N: usize, const TX_SZ: usize, const RX_SZ: usize> {
    inner: HttpClient<'static, TcpClient<'static, N, TX_SZ, RX_SZ>, DnsSocket<'static>>,
    // Own the resources that need to be dropped
    _dns: Box<DnsSocket<'static>>,
    _state: Box<TcpClientState<N, TX_SZ, RX_SZ>>,
    _tcp_client: Box<TcpClient<'static, N, TX_SZ, RX_SZ>>,
}

impl<const N: usize, const TX_SZ: usize, const RX_SZ: usize> Client<N, TX_SZ, RX_SZ> {
    pub async fn request<'conn>(
        &'conn mut self,
        method: Method,
        url: &'conn str,
    ) -> Result<
        HttpRequestHandle<
            'conn,
            <TcpClient<'static, N, TX_SZ, RX_SZ> as embedded_nal_async::TcpConnect>::Connection<
                'conn,
            >,
            (),
        >,
        Error,
    > {
        self.inner.request(method, url).await
    }
}

pub fn client<const N: usize, const TX_SZ: usize, const RX_SZ: usize>(
    stack: Stack<'static>,
    tls_reference: esp_mbedtls::TlsReference<'static>,
) -> Client<N, TX_SZ, RX_SZ> {
    let dns = Box::new(DnsSocket::new(stack.clone()));
    let mut certificates = Certificates::new();
    certificates.ca_chain = Some(
        X509::pem(crate::ca_certs::LETS_ENCRYPT_ISRG_ROOT_X1)
            .expect("Bug in CA certificate of lets encrypt. Failed to parse."),
    );
    let state = Box::new(TcpClientState::<N, TX_SZ, RX_SZ>::new());
    let state_ptr = state.as_ref() as *const TcpClientState<N, TX_SZ, RX_SZ>;
    // SAFETY: We ensure the boxed values live as long as the TcpClient
    let tcp_client = unsafe { Box::new(TcpClient::new(stack, &*state_ptr)) };

    // Get raw pointers for the HttpClient (if the API requires 'static references)
    let dns_ptr = dns.as_ref() as *const DnsSocket<'static>;
    let tcp_client_ptr = tcp_client.as_ref() as *const TcpClient<'static, N, TX_SZ, RX_SZ>;

    // SAFETY: We ensure the boxed values live as long as the Client struct
    let inner = unsafe {
        HttpClient::new_with_tls(
            &*tcp_client_ptr,
            &*dns_ptr,
            TlsConfig::new(reqwless::TlsVersion::Tls1_2, certificates, tls_reference),
        )
    };

    Client {
        inner,
        _dns: dns,
        _state: state,
        _tcp_client: tcp_client,
    }
}
