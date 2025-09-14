use std::{
    path::PathBuf,
    process::exit,
    sync::{atomic::{AtomicUsize, Ordering}, Arc},
};

use body_inspector::BoxBodyInspector;
use bytes::Bytes;
use crossterm::event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers};
use crossterm::terminal;
use hyper::{server::conn::http1, service::service_fn, Response};
use hyper_util::rt::TokioIo;
use logger::{update_stats, RequestInfo, StatsMsg};
use tls::{AcceptConnection, TlsWrapper, WithTls, WithoutTls};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::{broadcast, Notify};
use tokio::task::JoinHandle;

use crate::cli::CliArgs;


#[macro_use]
mod logger;
mod body_inspector;
mod html;
mod cli;
mod dir_to_zip;
mod local_response;
mod tls;
mod handle_response;

type BoxBodyResponse = Response<BoxBodyInspector<Bytes, std::io::Error>>;

static mut SHOW_HTML: bool = false;
static mut SPA_FILE: Option<PathBuf> = None;
static mut LOG_FILE: Option<PathBuf> = None;
const SERVER_NAME_HEADER: &str = "RustyPyserver";
const CHUNK_SIZE: usize = 96 * 1024; // bigger chunk == less update_stats(BytesSended)


#[tokio::main]
async fn main() {
    // Make space for the logger msgs
    println!();
    logger::init_stats_logger();
    let _ = terminal::enable_raw_mode();
  
    let cli_args = CliArgs::parse();
    unsafe { 
        SHOW_HTML = cli_args.show_html;
        LOG_FILE = cli_args.log_file;
    };

    // If the SPA file exists, set it to the global variable
    if let Some(spa_file_path) = cli_args.spa_file {
        if spa_file_path.exists() {
            print_info!("SPA file set to: {}", spa_file_path.display());
            unsafe { SPA_FILE = Some(spa_file_path) };
        } else {
            print_error!(0; "[--spa] File {} does not exist in the current dir", spa_file_path.display());
        }
    }

    let mut listeners = Vec::with_capacity(cli_args.listen_ports.len());
    let (shutdown_tx, _) = broadcast::channel::<()>(1);

    let protocol = match cli_args.tls {
        Some(_) => "https",
        None => "http"
    };

    let tls = match cli_args.tls {
        Some(tls_conf) => {
            match WithTls::new(tls_conf) {
                Ok(w) => TlsWrapper::With(w),
                Err(err) => {
                    print_error!("Failed to create TLS acceptor: {}", err);
                    exit(1);
                }
            }
        },
        None => TlsWrapper::Without(WithoutTls::default())
    };

    for port in cli_args.listen_ports {
        let addr = match cli_args.only_localhost {
            true => format!("localhost:{}", port),
            false => format!("0.0.0.0:{}", port), // [::] -> ipv6, 0.0.0.0 -> ipv4. For now, ipv4
        };

        let listener = match TcpListener::bind(&addr).await {
            Ok(l) => l,
            Err(e) => {
                print_error!("TcpListener bind error: {e}");
                continue;
            }
        };

        match cli_args.only_localhost {
            true => print_info!("Listening on {protocol}://localhost:{port}"),
            false => print_info!("Listening on {protocol}://localhost:{port} and {protocol}://{addr}")
        };

        let tls_cp = tls.clone();
        let mut shutdown_rx = shutdown_tx.subscribe();

        let handle: JoinHandle<anyhow::Result<()>> = tokio::spawn(async move {
            let mut request_info = RequestInfo::new(0, port);
            let active_requests = Arc::new(AtomicUsize::new(0));
            let notify = Arc::new(Notify::new());

            loop {
                tokio::select! {
                    Ok((stream, _)) = listener.accept() => {
                        let active_requests_cp = active_requests.clone();
                        let notify_cp = notify.clone();
                        handle_stream(stream, &tls_cp, request_info, active_requests_cp, notify_cp).await;
                        request_info.request_id += 1;
                    },
                    _ = shutdown_rx.recv() => {
                        if active_requests.load(Ordering::Relaxed) > 0 {
                            notify.notified().await;
                        }
                        print_info!("Shutting down server on {}", addr);
                        break Ok(())
                    }
                }
            }
        });

        listeners.push(handle);
    }

    let mut ctrl_c = KeyPress::new(KeyCode::Char('c'), KeyModifiers::CONTROL);
    let mut enter = KeyPress::new(KeyCode::Enter, KeyModifiers::empty());
    loop {
        if let Ok(Event::Key(e)) = crossterm::event::read() {
            if ctrl_c.pressing(&e) {
                print_info!("Shutdown signal received, shutting down gracefully... Press Ctrl + c again to force");
                let _ = terminal::disable_raw_mode();
                break
            }
            if enter.pressing(&e) {
                update_stats(StatsMsg::ShowDetails);
            }
        }
    }

    drop(shutdown_tx);
    for handle in listeners {
        let _ = handle.await;
    }
}

struct KeyPress {
    key: KeyCode,
    modifiers: KeyModifiers,
    released: bool
}
impl KeyPress {    
    pub fn new(key: KeyCode, modifiers: KeyModifiers) -> Self {
        Self { key, modifiers, released: true }
    }

    pub fn pressing(&mut self, e: &KeyEvent) -> bool {
        if e.code != self.key || e.modifiers != self.modifiers {
            return false;
        }

        let mut return_value = false;
        if self.released && e.kind == KeyEventKind::Press {
            return_value = true;
        }

        match e.kind {
            KeyEventKind::Release => self.released = true,
            KeyEventKind::Press => self.released = false,
            _ => {}
        }
        return return_value;
    }
}

async fn handle_stream(
    stream: TcpStream, 
    tls_cp: &TlsWrapper, 
    request_info: RequestInfo,
    active_requests: Arc<AtomicUsize>,
    notify: Arc<Notify>
) {
    let from_who = stream.peer_addr().unwrap_or("127.0.0.1:0".parse().unwrap());
    
    let stream = match tls_cp.accept(stream).await {
        Ok(s) => s,
        Err(e) => {
            print_error!("{e}");
            return;
        }
    };

    

    let io = TokioIo::new(stream);
    tokio::spawn(async move {
        active_requests.fetch_add(1, Ordering::Relaxed);
        update_stats(StatsMsg::NewRequest(request_info, from_who));

        if let Err(err) = http1::Builder::new()
            .serve_connection(io, service_fn(|req| handle_response::handle_response(req, from_who, request_info)))
            .await
        {
            print_error!("{} -> Failed to serve connection: {}", from_who, err.to_string());
        }

        if active_requests.fetch_sub(1, Ordering::Relaxed) == 1 {
            notify.notify_one();
        }
        update_stats(StatsMsg::RequestEnded(request_info));
    });
}