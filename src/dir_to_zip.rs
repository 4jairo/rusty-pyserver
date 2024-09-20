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

pub async fn dir_to_zip(dir: impl AsRef<str>) -> HyperResult<BoxBodyResponse> {
    let (a, b) = tokio::io::duplex(CHUNK_SIZE);
    
    let dir = dir.as_ref();
    let dir_clone = dir.to_string();
    tokio::spawn(async move {
        let mut archive = Archive::new(a);

        for entry in WalkDir::new(&dir_clone) {
            let Ok(entry) = entry else {
                continue;
            };
            if entry.file_type().is_dir() {
                continue;
            }
    
            let path = entry.path();
            let name = parse_path_name(path.strip_prefix(&dir_clone).unwrap().to_string_lossy());
            let mut file = match File::open(path).await {
                Err(_err) => continue, //Some(error_response(err.to_string())), // panic!("\n{}\n", err),
                Ok(file) => file,
            };
            
            let systemtime = match file.metadata().await {
                Err(_err) => SystemTime::now(),
                Ok(m) => m.modified().unwrap_or(SystemTime::now())
            };
            let datetime = DateTime::<Local>::from(systemtime);
            let datetime = FileDateTime::from_chrono_datetime(datetime);

            if let Err(_err) = archive.append(name, datetime, &mut file).await {
                continue; //Some(error_response(err.to_string())); // panic!("\n{}\n", err)
            }
        }

        if let Err(_err) = archive.finalize().await {
            return //Some(error_response(err.to_string())); // panic!("\n{}\n", err)
        }
       //None
    });

    let reader_stream = ReaderInspector::new(ReaderStream::new(b));
    let body = StreamBody::new(reader_stream.map_ok(Frame::data)).boxed();
    let zip_name = match dir {
        "." => "result".to_string(),
        _ => format!("{}", dir.replace("/", "_")),
    };

    let response = Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, "application/zip")
        .header(CONTENT_DISPOSITION, format!("attachment; filename={}.zip", zip_name))
        .header(SERVER, SERVER_NAME_HEADER)
        .body(body)
        .unwrap();

    Ok(response)
}