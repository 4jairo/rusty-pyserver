use std::{pin::Pin, task::{Context, Poll}};

use bytes::Buf;
use http_body_util::combinators::BoxBody;
use hyper::body::{Body, Frame};

use crate::logger::{update_stats, RequestInfo, StatsMsg};

pub struct BoxBodyInspector<D, E> {
    inner: BoxBody<D, E>,
    file: String,
    request_info: RequestInfo
}

impl<D, E> BoxBodyInspector<D, E> {
    pub fn new(inner: BoxBody<D, E>, file: String, request_info: RequestInfo) -> Self {
        Self { inner, file, request_info }
    }
}

impl<D, E> Body for BoxBodyInspector<D, E> 
where
    D: Buf,
{
    type Data = D;
    type Error = E;

    fn poll_frame(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Result<Frame<Self::Data>, Self::Error>>> {
        let r = Pin::new(&mut self.inner).poll_frame(cx);

        if let Poll::Ready(Some(Ok(chunk))) = &r {
            if let Some(data) = chunk.data_ref() {
                update_stats(StatsMsg::SendedBytes(
                    self.request_info, 
                    data.remaining() as u32,
                    self.file.clone()
                ));
            }
        }

        r
    }

    fn is_end_stream(&self) -> bool {
        self.inner.is_end_stream()
    }

    fn size_hint(&self) -> hyper::body::SizeHint {
        self.inner.size_hint()
    }
}

impl<D, E> Drop for BoxBodyInspector<D, E> {
    fn drop(&mut self) {
        update_stats(StatsMsg::EndedFile(self.request_info, self.file.clone()));
    }
}