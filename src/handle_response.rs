use std::{net::SocketAddr, path::{Path, PathBuf}};
use askama::Template;
use futures_util::TryStreamExt;
use http_body_util::{BodyExt, StreamBody};
use hyper::{body::{Frame, Incoming}, header::{CONTENT_LENGTH, CONTENT_TYPE, SERVER}, Request, Response, Result as HyperResult, StatusCode};
use serde::Deserialize;
use tokio::fs::{self, File};
use tokio_util::io::ReaderStream;
use crate::{body_inspector::BoxBodyInspector, dir_to_zip, html::{format_file_size, DirectoryFile, HtmlTemplate}, local_response::{index, not_found}, logger::{print_request, RequestInfo, RequestKind}, BoxBodyResponse, CHUNK_SIZE, SERVER_NAME_HEADER, SHOW_HTML, SPA_FILE};

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
    let path_raw = urlencoding::decode(req.uri().path()).unwrap_or_default();
    let method = req.method().clone();

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
        let template = HtmlTemplate::new(path_raw, files_in_curr_path).unwrap();
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