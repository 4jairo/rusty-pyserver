use bytes::Bytes;
use futures_util::TryStreamExt;
use http_body_util::combinators::BoxBody;
use http_body_util::{BodyExt, StreamBody};
use hyper::header::{CONTENT_LENGTH, CONTENT_TYPE, SERVER};
use hyper_util::rt::TokioIo;
use local_response::{index, not_found};
use logger::{update_stats, StatsMsg};
use reader_inspector::ReaderInspector;
use tokio_util::io::ReaderStream;
use hyper::{
    body::Frame,
    server::conn::http1,
    service::service_fn,
    Result as HyperResult,
    body::Incoming,
    Request,
    Response,
    StatusCode,
};
use tokio::{
    task::JoinHandle,
    fs::{self, File},
    net::TcpListener,
};
use std::{
    net::SocketAddr,
    path::{Path, PathBuf},
};
use crate::{
    cli::CliArgs,
    html::{format_file_size, DirectoryFile, build_html2},
};

#[macro_use]
mod logger;
mod reader_inspector;
mod html;
mod cli;
mod dir_to_zip;
mod local_response;

type BoxBodyResponse = Response<BoxBody<Bytes, std::io::Error>>;

static mut SHOW_HTML: bool = false;
static mut SPA_FILE: Option<PathBuf> = None;
static mut LOG_FILE: Option<PathBuf> = None;
const SERVER_NAME_HEADER: &str = "RustyPyserver";
const CHUNK_SIZE: usize = 32 * 1024;


#[tokio::main]
async fn main() {
    // Make space for the logger msgs
    println!();
    logger::init_stats_logger();

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

    for port in cli_args.listen_ports {
        let addr = match cli_args.only_localhost {
            true => format!("localhost:{}", port),
            false => format!("0.0.0.0:{}", port),
        };

        let listener = match TcpListener::bind(&addr).await {
            Ok(l) => l,
            Err(e) => {
                print_error!("TcpListener bind error: {e}");
                continue;
            }
        };

        match cli_args.only_localhost {
            true => print_info!("Listening on http://localhost:{}", port),
            false => print_info!("Listening on http://localhost:{} and http://{}", port, addr)
        };

        let handle: JoinHandle<anyhow::Result<()>> = tokio::spawn(async move {
            loop {
                let Ok((stream, _)) = listener.accept().await else {
                    continue;
                };

                let from_who = stream.peer_addr().unwrap();
                let io = TokioIo::new(stream);
                
                tokio::spawn(async move {
                    update_stats(StatsMsg::NewRequest);
                    if let Err(err) = http1::Builder::new()
                        .serve_connection(io, service_fn(|req| handle_response(req, from_who, port)))
                        .await
                    {
                        print_error!("{} -> Failed to serve connection: {:?}", from_who, err);
                    }
                    update_stats(StatsMsg::RequestEnded);
                });
            }
        });

        listeners.push(handle);
    }

    for handle in listeners {
        let _ = handle.await;
    }
}


async fn handle_response(req: Request<Incoming>, who: SocketAddr, port: u16) -> HyperResult<BoxBodyResponse> {
    let path_raw = urlencoding::decode(req.uri().path()).unwrap_or_default();

    let method = req.method();
    let now = chrono::Local::now().format("%d-%m-%Y %H:%M:%S");
    print_request!(":{port} [{now}] --> {who} --> {method} {path_raw}");

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

        return dir_to_zip::dir_to_zip(path).await;
    }

    let path_metadata = match fs::metadata(path).await {
        Ok(metadata) => metadata,
        Err(_) => return Ok(not_found()),
    };

    if path_metadata.is_file() {
        return file_send(path, path_metadata.len() as usize).await
    }

    unsafe {
        // If the SPA file exists, serve it
        if let Some(spa_file) = SPA_FILE.as_ref() {
            let metadata = match spa_file.metadata() {
                Ok(m) => m,
                Err(e) => {
                    print_error!("Error reading SPA file metadata: {e}");
                    return Ok(not_found());
                }
            };

            return file_send(spa_file, metadata.len() as usize).await;
        }

        // If the --html flag is set, serve the index.html file
        if SHOW_HTML {
            let html_path = Path::new(path).join("index.html");
            if let Ok(metadata) = html_path.metadata() {
                return file_send(html_path, metadata.len() as usize).await;
            }
        }
    }

    let files_in_curr_path = match get_files_in_dir2(path) {
        Ok(files) => files,
        Err(_) => return Ok(not_found()),
    };

    let html = build_html2(path_raw, files_in_curr_path);
    update_stats(StatsMsg::SendedBytes(html.len() as u32));
    Ok(index(html))
}


fn get_files_in_dir2(path: impl AsRef<Path>) -> Result<Vec<DirectoryFile>, std::io::Error> {
    let result = std::fs::read_dir(path)?
        .filter_map(|e| {
            match e {
                Err(_) => None,
                Ok(e) => {
                    let is_dir = match e.file_type() {
                        Ok(t) => t.is_dir(),
                        Err(_) => false
                    };

                    let file_name = match is_dir {
                        true => format!("{}/", e.path().file_name().unwrap().to_str().unwrap()),
                        false => e.path().file_name().unwrap().to_str().unwrap().to_string()
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

    Ok(result)
}


async fn file_send(filename: impl AsRef<Path>, file_len: usize) -> HyperResult<BoxBodyResponse> {
    //Wrap to a tokio_util::io::ReaderStream
    let reader_stream = match File::open(&filename).await {
        Ok(file) => ReaderInspector::new(
            ReaderStream::with_capacity(file, CHUNK_SIZE)
        ),
        Err(_) => return Ok(not_found()),
    };

    let mime = unsafe {
        match SHOW_HTML {
            true => mime_guess::from_path(&filename).first_or_text_plain(),
            false => mime_guess::mime::APPLICATION_OCTET_STREAM
        }
    };

    // Convert to http_body_util::BoxBody
    let stream_body = StreamBody::new(reader_stream.map_ok(Frame::data)).boxed();

    let response = Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, mime.to_string())
        .header(CONTENT_LENGTH, file_len)
        .header(SERVER, SERVER_NAME_HEADER)
        .body(stream_body)
        .unwrap();

    Ok(response)
}
