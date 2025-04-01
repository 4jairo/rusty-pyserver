use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
    process::exit,
    sync::{atomic::{AtomicUsize, Ordering}, Arc},
};
use askama::Template;
use body_inspector::BoxBodyInspector;
use bytes::Bytes;
use crossterm::{event::{Event, KeyCode, KeyEvent, KeyEventKind, KeyModifiers}, terminal};
use futures_util::TryStreamExt;
use http_body_util::{BodyExt, StreamBody};
use hyper::{
    body::{Frame, Incoming},
    server::conn::http1,
    service::service_fn,
    Request,
    Response,
    Result as HyperResult,
    StatusCode,
};
use hyper::header::{CONTENT_LENGTH, CONTENT_TYPE, SERVER};
use hyper_util::rt::TokioIo;
use logger::{update_stats, RequestInfo, StatsMsg};
use local_response::{index, not_found};
use serde::Deserialize;
use tls::{AcceptConnection, TlsWrapper, WithTls, WithoutTls};
use tokio::{
    fs::{self, File},
    net::{TcpListener, TcpStream},
    sync::{broadcast, Notify},
    task::JoinHandle,
};
use tokio_util::io::ReaderStream;
use crate::{
    cli::CliArgs,
    html::{format_file_size, DirectoryFile, HtmlTemplate},
};


#[macro_use]
mod logger;
mod body_inspector;
mod html;
mod cli;
mod dir_to_zip;
mod local_response;
mod tls;

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
            .serve_connection(io, service_fn(|req| handle_response(req, from_who, request_info)))
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

#[derive(Deserialize, Debug)]
struct QueryParams {
    files: String
}

async fn handle_response(req: Request<Incoming>, who: SocketAddr, req_info: RequestInfo) -> HyperResult<BoxBodyResponse> {
    let path_raw = urlencoding::decode(req.uri().path()).unwrap_or_default();

    let method = req.method();
    let now = chrono::Local::now().format("%d-%m-%Y %H:%M:%S");
    print_request!("[{now}] {who} --> :{} --> {method} {path_raw}", req_info.listener);

    let path = match path_raw.len() {
        1 => ".", // If the path is just '/', serve the current directory
        _ => &path_raw[1..],
    };

    // If the path starts with '*', it means we want to zip the directory
    if path.starts_with("*") {
        let path = match path {
            "*" | "*/" => ".",
            _ => &path[2..],
        };

        let query_raw = urlencoding::decode(req.uri().query().unwrap_or_default()).unwrap_or_default();
        let files = match serde_qs::from_str::<QueryParams>(&query_raw) {
            Ok(files_json) => match serde_json::from_str::<Vec<String>>(&files_json.files) {
                Ok(f) => f,
                Err(_) => vec![]
            },
            Err(_) => vec![]
        };

        return dir_to_zip::dir_to_zip(path, files, req_info).await;
    }

    let path_metadata = match fs::metadata(path).await {
        Ok(metadata) => metadata,
        Err(_) => return Ok(not_found(req_info)),
    };

    if path_metadata.is_file() {
        return file_send(path, path_metadata.len() as usize, req_info).await
    }

    unsafe {
        // If the SPA file exists, serve it
        if let Some(spa_file) = SPA_FILE.as_ref() {
            let metadata = match spa_file.metadata() {
                Ok(m) => m,
                Err(e) => {
                    print_error!("Error reading SPA file metadata: {e}");
                    return Ok(not_found(req_info));
                }
            };

            return file_send(spa_file, metadata.len() as usize, req_info).await;
        }

        // If the --html flag is set, serve the index.html file
        if SHOW_HTML {
            let html_path = Path::new(path).join("index.html");
            if let Ok(metadata) = html_path.metadata() {
                return file_send(html_path, metadata.len() as usize, req_info).await;
            }
        }
    }

    let files_in_curr_path = match get_files_in_dir2(path) {
        Ok(files) => files,
        Err(_) => return Ok(not_found(req_info)),
    };
    
    let template = HtmlTemplate::new(path_raw, files_in_curr_path).unwrap();
    let html = template.render().unwrap();
    Ok(index(html, req_info)) 
}


fn get_files_in_dir2(path: impl AsRef<Path>) -> Result<Vec<DirectoryFile>, std::io::Error> {
    let mut result = std::fs::read_dir(path)?
        .filter_map(|e| {
            match e {
                Err(_) => None,
                Ok(e) => {
                    let is_dir = match e.file_type() {
                        Ok(t) => t.is_dir(),
                        Err(_) => false
                    };

                    let file_name = match is_dir {
                        true => format!("{}/", e.path().file_name().unwrap_or_default().to_string_lossy()),
                        false => e.path().file_name().unwrap_or_default().to_string_lossy().to_string()
                    };

                    let file_size = match is_dir {
                        true => "".to_string(),
                        false => format_file_size(
                            e.metadata().map(|m| m.len()).unwrap_or_default()
                        )
                    };
                
                    Some(DirectoryFile { is_dir, file_size, file_name })
                }
            }
        })
        .collect::<Vec<_>>();

    result.sort_by(|a,b| natord::compare(&a.file_name, &b.file_name));
    Ok(result)
}


async fn file_send(filename: impl AsRef<Path>, file_len: usize, req_info: RequestInfo) -> HyperResult<BoxBodyResponse> {
    let reader_stream = match File::open(&filename).await {
        Ok(file) => ReaderStream::with_capacity(file, CHUNK_SIZE),
        Err(_) => return Ok(not_found(req_info)),
    };

    let mime = unsafe {
        match SHOW_HTML {
            true => mime_guess::from_path(&filename).first_or_text_plain(),
            false => mime_guess::mime::APPLICATION_OCTET_STREAM
        }
    };

    let stream_body = BoxBodyInspector::new(
        StreamBody::new(reader_stream.map_ok(Frame::data)).boxed(),
        filename.as_ref().to_string_lossy().to_string(),
        req_info
    );

    let response = Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, mime.to_string())
        .header(CONTENT_LENGTH, file_len)
        .header(SERVER, SERVER_NAME_HEADER)
        .body(stream_body)
        .unwrap();

    Ok(response)
}
