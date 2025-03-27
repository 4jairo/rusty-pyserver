use std::borrow::Cow;
use std::time::SystemTime;

use chrono::{DateTime, Local};
use futures_util::TryStreamExt;
use http_body_util::{BodyExt, StreamBody};
use hyper::body::Frame;
use hyper::header::{CONTENT_DISPOSITION, CONTENT_TYPE, SERVER};
use hyper::{Response, StatusCode, Result as HyperResult};
use tokio::fs::File;
use tokio_util::io::ReaderStream;
use walkdir::WalkDir;
use zipit::{Archive, FileDateTime};
use crate::logger::RequestInfo;
use crate::reader_inspector::ReaderInspector;
use crate::{BoxBodyResponse, CHUNK_SIZE, SERVER_NAME_HEADER};

fn parse_path_name(path: Cow<str>) -> String {
    #[cfg(windows)]
    {
        path.replace("\\", "/")
    }

    #[cfg(not(windows))]
    {
        path.to_string()
    }
}

pub async fn dir_to_zip(dir: impl AsRef<str>, files: Vec<String>, request_info: RequestInfo) -> HyperResult<BoxBodyResponse> {
    let (a, b) = tokio::io::duplex(CHUNK_SIZE);
    
    let dir = dir.as_ref();
    let dir_clone = dir.to_string();
    tokio::spawn(async move {
        let mut archive = Archive::new(a);

        
        for entry_dir in WalkDir::new(&dir_clone).max_depth(1).min_depth(1) {
            let Ok(entry_dir) = entry_dir else {
                continue;
            }; 
            if !files.is_empty() && !files.iter().any(|f| entry_dir.file_name().to_str() == Some(f.as_str())) {
                continue;
            }

 

            for entry in WalkDir::new(&entry_dir.path()) {
                let Ok(entry) = entry else {
                    continue;
                };
                if entry.file_type().is_dir() {
                    continue;
                }
        
                let path = entry.path();
                let name = parse_path_name(path.strip_prefix(&dir_clone).unwrap().to_string_lossy());
                let mut file = match File::open(path).await {
                    Err(_err) => continue,
                    Ok(file) => file,
                };
                
                let systemtime = match file.metadata().await {
                    Err(_err) => SystemTime::now(),
                    Ok(m) => m.modified().unwrap_or(SystemTime::now())
                };
                let datetime = DateTime::<Local>::from(systemtime);
                let datetime = FileDateTime::from_chrono_datetime(datetime);
    
                if let Err(_err) = archive.append(name, datetime, &mut file).await {
                    continue;
                }
            }
        }
        

        if let Err(_err) = archive.finalize().await {
            return
        }
    });

    let zip_name = match dir {
        "." => "result.zip".to_string(),
        _ => {
            let mut zip_name = format!("{}.zip", dir.replace("/", "_"));
            if zip_name.ends_with("_") {
                zip_name.pop();
            }
            zip_name
        },
    };
    let reader_stream = ReaderInspector::new(ReaderStream::with_capacity(b, CHUNK_SIZE), zip_name.clone(), request_info);
    let body = StreamBody::new(reader_stream.map_ok(Frame::data)).boxed();
 
    let response = Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, "application/zip")
        .header(CONTENT_DISPOSITION, format!("attachment; filename={}", zip_name))
        .header(SERVER, SERVER_NAME_HEADER)
        .body(body)
        .unwrap();

    Ok(response)
}