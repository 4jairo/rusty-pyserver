use std::{io, sync::Arc};

use rustls::pki_types::{pem::PemObject, CertificateDer, PrivateKeyDer};
use tokio::{io::{AsyncRead, AsyncWrite}, net::TcpStream};
use tokio_rustls::TlsAcceptor;

pub trait AsyncStream: AsyncRead + AsyncWrite + Unpin + Send {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send> AsyncStream for T {}

pub trait AcceptConnection {
    async fn accept(&self, stream: TcpStream) -> Result<Box<dyn AsyncStream>, io::Error>;
}

pub enum TlsConfig {
    Generated(Vec<String>),
    Provided(String, String)
}

#[derive(Clone)]
pub struct WithTls {
    acceptor: TlsAcceptor
}

impl WithTls {
    pub fn new(tls_conf: TlsConfig) -> anyhow::Result<Self> { 
        let (certs, key) = match tls_conf {
            TlsConfig::Generated(domains) => {
                print_info!("{:?}", domains);
                let certs = rcgen::generate_simple_self_signed(domains)?;
                let cert_der = CertificateDer::from(certs.cert);
                let key_der = PrivateKeyDer::try_from(certs.key_pair.serialize_der()).map_err(|e| anyhow::anyhow!(e))?;

                (vec![cert_der], key_der)
            },
            TlsConfig::Provided(cert_path, key_path) => {
                let certs = CertificateDer::pem_file_iter(cert_path)?.collect::<Result<Vec<_>, _>>()?;
                let key = PrivateKeyDer::from_pem_file(key_path)?;

                (certs, key)
            }
        };

        let config = rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(certs, key)?;

        let acceptor = TlsAcceptor::from(Arc::new(config));

        Ok(Self { acceptor })
    }
}

impl AcceptConnection for WithTls {
    async fn accept(&self, stream: TcpStream) -> Result<Box<dyn AsyncStream>, io::Error> {
        let tls_stream = self.acceptor.accept(stream).await?;
        Ok(Box::new(tls_stream))
    }
}

#[derive(Default, Clone)]
pub struct WithoutTls;

impl AcceptConnection for WithoutTls {
    async fn accept(&self, stream: TcpStream) -> Result<Box<dyn AsyncStream>, io::Error> {
        Ok(Box::new(stream))
    }
}

#[derive(Clone)]
pub enum TlsWrapper {
    With(WithTls),
    Without(WithoutTls)
}
impl AcceptConnection for TlsWrapper {
    async fn accept(&self, stream: TcpStream) -> Result<Box<dyn AsyncStream>, io::Error> {
        match self {
            TlsWrapper::With(tls) => tls.accept(stream).await,
            TlsWrapper::Without(wihtout) => wihtout.accept(stream).await
        }
    }
}