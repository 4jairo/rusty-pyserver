use std::{pin::Pin, task::{Context, Poll}};

use futures_util::Stream;
use pin_project_lite::pin_project;
use tokio::io::AsyncRead;
use tokio_util::io::ReaderStream;

use crate::logger::{update_stats, StatsMsg};


pin_project! {
    pub struct ReaderInspector<R: AsyncRead> {
        #[pin]
        r: ReaderStream<R>
    }
}
impl<R: AsyncRead> ReaderInspector<R> {
    pub fn new(r: ReaderStream<R>) -> Self {
        Self { r }
    }
}

impl<R: AsyncRead> Stream for ReaderInspector<R> {
    type Item = <ReaderStream<R> as Stream>::Item;

    fn poll_next(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        let r = self.project().r.poll_next(cx);

        if let Poll::Ready(Some(Ok(chunk))) = &r {
            update_stats(StatsMsg::SendedBytes(chunk.len() as u32));
        }

        r
    }
} 
