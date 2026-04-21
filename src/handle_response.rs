use std::{net::SocketAddr, path::{Component, Path, PathBuf}};
use askama::Template;
use bytes::Bytes;
use futures_util::TryStreamExt;
use http_body_util::{BodyExt, Full, StreamBody};
use hyper::{Method, Request, Response, Result as HyperResult, StatusCode, body::{Frame, Incoming}, header::{CONTENT_LENGTH, CONTENT_TYPE, SERVER}};
use multer::Multipart;
use serde::Deserialize;
use tokio::fs::{self, File};
use tokio::io::AsyncWriteExt;
use tokio_util::io::ReaderStream;
use crate::{body_inspector::BoxBodyInspector, dir_to_zip, html::{format_file_size, DirectoryFile, HtmlTemplate}, local_response::{index, not_found}, logger::{print_request, RequestInfo, RequestKind}, BoxBodyResponse, CHUNK_SIZE, ENABLE_UPLOAD, SERVER_NAME_HEADER, SHOW_HTML, SPA_FILE};

#[derive(Deserialize, Debug)]
struct QueryParams {
    files: String
}

/*
    *.... -> dir_to_zip,
    if fs::metadata(path) 
        is_file -> file_send(file),
        SPA_FILE -> file_send(spa_file)
        SHOW_HTML -> file_send(dir + /index.html)
        else -> get_files_in_dir(dir)
    else 
        SPA_FILE -> file_send(spa_file)
*/

pub async fn handle_response(req: Request<Incoming>, who: SocketAddr, req_info: RequestInfo) -> HyperResult<BoxBodyResponse> {
    let path_raw: std::borrow::Cow<'static, str> = std::borrow::Cow::Owned(
        urlencoding::decode(req.uri().path()).unwrap_or_default().into_owned()
    );
    let method = req.method().clone();

    let path = match path_raw.len() {
        1 => ".", // If the path is just '/', serve the current directory
        _ => &path_raw[1..],
    };

    if method == Method::POST {
        if unsafe { !ENABLE_UPLOAD } {
            print_request(RequestKind::NotFound, who, method, &path_raw, req_info.listener);
            return Ok(not_found(req_info));
        }

        return upload_file(req, path, who, method, &path_raw, req_info).await;
    }

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

        print_request(RequestKind::DirToZip, who, method, &path_raw, req_info.listener);
        return dir_to_zip::dir_to_zip(path, files, req_info).await;
    }

    if let Ok(path_metadata) = fs::metadata(path).await {
        if path_metadata.is_file() {
            print_request(RequestKind::Default, who, method, &path_raw, req_info.listener);
            return file_send(path, path_metadata.len() as usize, req_info).await
        }

        if let Some(spa_file) = unsafe { SPA_FILE.as_ref() } {
            print_request(RequestKind::Spa, who, method, &path_raw, req_info.listener);
            return handle_spa(spa_file, req_info).await
        }

        if unsafe { SHOW_HTML } {
            let html_path = Path::new(path).join("index.html");
            if let Ok(metadata) = html_path.metadata() {
                print_request(RequestKind::Html, who, method, &path_raw, req_info.listener);
                return file_send(html_path, metadata.len() as usize, req_info).await;
            }
        }

        let files_in_curr_path = match get_files_in_dir(path) {
            Ok(files) => files,
            Err(_) => {
                print_request(RequestKind::NotFound, who, method, &path_raw, req_info.listener);
                return Ok(not_found(req_info))
            },
        };
        
        print_request(RequestKind::Default, who, method, &path_raw, req_info.listener);
        let template = HtmlTemplate::new(path_raw, files_in_curr_path, unsafe { ENABLE_UPLOAD }).unwrap();
        let html = template.render().unwrap();
        return Ok(index(html, req_info))
    }

    if let Some(spa_file) = unsafe { SPA_FILE.as_ref() } {
        print_request(RequestKind::Spa, who, method, &path_raw, req_info.listener);
        return handle_spa(spa_file, req_info).await
    }

    print_request(RequestKind::NotFound, who, method, &path_raw, req_info.listener);
    Ok(not_found(req_info))
}

async fn upload_file(
    req: Request<Incoming>,
    path: &str,
    who: SocketAddr,
    method: Method,
    path_raw: &std::borrow::Cow<'_, str>,
    req_info: RequestInfo,
) -> HyperResult<BoxBodyResponse> {
    match fs::metadata(path).await {
        Ok(m) if m.is_dir() => m,
        _ => return Ok(not_found(req_info)),
    };

    let content_type = match req.headers().get(CONTENT_TYPE).and_then(|v| v.to_str().ok()) {
        Some(c) => c.to_string(),
        None => return Ok(text_response(StatusCode::BAD_REQUEST, "Missing Content-Type", req_info)),
    };

    let boundary = match multer::parse_boundary(&content_type) {
        Ok(b) => b,
        Err(_) => return Ok(text_response(StatusCode::BAD_REQUEST, "Invalid multipart boundary", req_info)),
    };

    let body_stream = req
        .into_body()
        .into_data_stream()
        .map_err(|e| std::io::Error::other(e.to_string()));
    let mut multipart = Multipart::new(body_stream, boundary);

    let mut uploaded_names = Vec::new();
    loop {
        let next_field = match multipart.next_field().await {
            Ok(field) => field,
            Err(e) => {
                print_error!("Error reading multipart field: {e}");
                return Ok(text_response(StatusCode::BAD_REQUEST, "Invalid multipart body", req_info));
            }
        };

        let mut field = match next_field {
            Some(field) => field,
            None => break,
        };

        let filename = match field.file_name().and_then(sanitize_filename) {
            Some(name) => name.to_string(),
            None => continue,
        };

        let file_path = Path::new(path).join(&filename);
        let mut out_file = match tokio::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&file_path)
            .await
        {
            Ok(file) => file,
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(e) => {
                print_error!("Error creating uploaded file {}: {e}", file_path.to_string_lossy());
                return Ok(text_response(StatusCode::INTERNAL_SERVER_ERROR, "Failed to create uploaded file", req_info));
            }
        };

        loop {
            let chunk = match field.chunk().await {
                Ok(chunk) => chunk,
                Err(e) => {
                    print_error!("Error streaming multipart chunk: {e}");
                    return Ok(text_response(StatusCode::BAD_REQUEST, "Failed while reading upload stream", req_info));
                }
            };

            let Some(chunk) = chunk else {
                break;
            };

            if let Err(e) = out_file.write_all(&chunk).await {
                print_error!("Error writing uploaded file {}: {e}", file_path.to_string_lossy());
                return Ok(text_response(StatusCode::INTERNAL_SERVER_ERROR, "Failed to write uploaded file", req_info));
            }
        }

        uploaded_names.push(filename);
    }

    if uploaded_names.is_empty() {
        return Ok(text_response(StatusCode::BAD_REQUEST, "No valid files uploaded", req_info));
    }

    let log_details = format!("{} file(s): {}", uploaded_names.len(), uploaded_names.join(", "));
    print_request(RequestKind::Upload(log_details), who, method, path_raw, req_info.listener);

    Ok(text_response(StatusCode::CREATED, format!("Uploaded {} file(s)", uploaded_names.len()), req_info))
}

fn sanitize_filename(filename: &str) -> Option<&str> {
    if filename.is_empty() {
        return None;
    }

    let mut components = Path::new(filename).components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(_)), None) => Some(filename),
        _ => None,
    }
}

async fn handle_spa(spa_file: &PathBuf, req_info: RequestInfo) -> HyperResult<BoxBodyResponse> {
    let metadata = match spa_file.metadata() {
        Ok(m) => m,
        Err(e) => {
            print_error!("Error reading SPA file metadata: {e}");
            return Ok(not_found(req_info));
        }
    };

    return file_send(spa_file, metadata.len() as usize, req_info).await;
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

fn text_response(status: StatusCode, body: impl Into<Bytes>, req_info: RequestInfo) -> BoxBodyResponse {
    let bytes: Bytes = body.into();
    let bytes_len = bytes.len();

    let body_inner = Full::new(bytes)
        .map_err(|never| match never {})
        .boxed();

    let body = BoxBodyInspector::new(body_inner, "*upload*".to_string(), req_info);
    Response::builder()
        .status(status)
        .header(CONTENT_TYPE, "text/plain; charset=utf-8")
        .header(SERVER, SERVER_NAME_HEADER)
        .header(CONTENT_LENGTH, bytes_len)
        .body(body)
        .unwrap()
}


fn get_files_in_dir(path: impl AsRef<Path>) -> Result<Vec<DirectoryFile>, std::io::Error> {
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