
use bytes::Bytes;
use http_body_util::{BodyExt, Full};
use hyper::{Response, StatusCode, header::{CONTENT_LENGTH, CONTENT_TYPE, SERVER}};
use crate::{BoxBodyResponse, SERVER_NAME_HEADER};

 
/// HTTP status code 404
pub fn not_found() -> BoxBodyResponse {
    let body = Full::new("404 Not Found".into())
        .map_err(|never| match never {})
        .boxed();

    Response::builder()
        .header(SERVER, SERVER_NAME_HEADER)
        .status(StatusCode::NOT_FOUND)
        .body(body)
        .unwrap()
}

/// HTTP status code 500
// pub fn error_response(msg: impl Into<Bytes>) -> BoxBodyResponse {
//     let body = Full::new(msg.into())
//         .map_err(|never| match never {})
//         .boxed();

//     Response::builder()
//         .status(StatusCode::INTERNAL_SERVER_ERROR)
//         .body(body)
//         .unwrap()
// }

pub fn index(index: impl Into<Bytes>) -> BoxBodyResponse {
    let bytes: Bytes = index.into();
    let bytes_len = bytes.len();

    let body = Full::new(bytes)
        .map_err(|never| match never {})
        .boxed();

    Response::builder()
        .status(StatusCode::OK)
        .header(CONTENT_TYPE, "text/html")
        .header(SERVER, SERVER_NAME_HEADER)
        .header(CONTENT_LENGTH, bytes_len)
        .body(body)
        .unwrap()
}
