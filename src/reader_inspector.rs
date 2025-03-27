use std::{pin::Pin, task::{Context, Poll}};

use futures_util::Stream;
use pin_project_lite::pin_project;
use tokio::io::AsyncRead;
use tokio_util::io::ReaderStream;
use crate::logger::{update_stats, RequestInfo, StatsMsg};

pin_project! {
    pub struct ReaderInspector<R: AsyncRead> {
        #[pin]
        r: ReaderStream<R>,
        file: String,
        request_info: RequestInfo
    }
}
impl<R: AsyncRead> ReaderInspector<R> {
    pub fn new(r: ReaderStream<R>, file: impl Into<String>, request_id: RequestInfo) -> Self {
        let reader = Self { 
            r,
            file: file.into(),
            request_info: request_id
        }; 

        reader
    }
}

impl<R: AsyncRead> Stream for ReaderInspector<R> {
    type Item = <ReaderStream<R> as Stream>::Item;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context) -> Poll<Option<Self::Item>> {
        let request_info = self.request_info.clone();
        let file = self.file.clone();

        let r = self.project().r.poll_next(cx);

        if let Poll::Ready(Some(Ok(chunk))) = &r {
            update_stats(StatsMsg::SendedBytes(request_info, chunk.len() as u32, file));
        }

        r
    }
}