use std::pin::Pin;
use std::task::{Context, Poll};
use std::sync::Arc;
use std::time::Instant;
use bytes::Bytes;
use futures_util::Stream;
use axum::body::Body;
use axum::response::{Response};

use crate::cost::{SSEUsageAccumulator, CostTracker};
use crate::audit::{CompressionAudit, AuditLogger};

pub struct LoggingStream<S> {
    inner_stream: S,
    sse_acc: Option<SSEUsageAccumulator>,
    request_audit: CompressionAudit,
    path: String,
    method: String,
    request_model: Option<String>,
    bytes_in: usize,
    bytes_out: usize,
    started_perf: Instant,
    audit_logger: Arc<AuditLogger>,
    cost_tracker: Arc<CostTracker>,
    status_code: u16,
    request_id: Option<String>,
    logged: bool,
}

impl<S> Stream for LoggingStream<S>
where
    S: Stream<Item = Result<Bytes, reqwest::Error>> + Unpin,
{
    type Item = Result<Bytes, std::io::Error>;

    fn poll_next(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<Option<Self::Item>> {
        match Pin::new(&mut self.inner_stream).poll_next(cx) {
            Poll::Ready(Some(Ok(chunk))) => {
                self.bytes_out += chunk.len();
                if let Some(ref mut acc) = self.sse_acc {
                    acc.feed(&chunk);
                }
                Poll::Ready(Some(Ok(chunk)))
            }
            Poll::Ready(Some(Err(e))) => {
                Poll::Ready(Some(Err(std::io::Error::new(std::io::ErrorKind::Other, e))))
            }
            Poll::Ready(None) => {
                self.record_final_stats();
                Poll::Ready(None)
            }
            Poll::Pending => Poll::Pending,
        }
    }
}

impl<S> LoggingStream<S> {
    fn record_final_stats(&mut self) {
        if self.logged {
            return;
        }
        self.logged = true;

        let mut final_model = self.request_model.clone();
        if let Some(ref acc) = self.sse_acc {
            if acc.saw_message_start() {
                if let Some(m) = acc.model() {
                    final_model = Some(m.to_string());
                }
                let usage = acc.snapshot();
                let cost_record = crate::cost::estimate(
                    "anthropic",
                    final_model.as_deref().unwrap_or(""),
                    *usage.get("input_tokens").unwrap_or(&0),
                    *usage.get("output_tokens").unwrap_or(&0),
                    *usage.get("cache_write_tokens").unwrap_or(&0),
                    *usage.get("cache_read_tokens").unwrap_or(&0),
                );
                self.cost_tracker.record(&cost_record);
            }
        }

        let latency_ms = self.started_perf.elapsed().as_secs_f64() * 1000.0;
        self.audit_logger.record(
            &self.method,
            &self.path,
            Some(self.status_code),
            &self.request_audit,
            None,
            self.request_id.as_deref(),
            None,
            Some(latency_ms),
            self.bytes_in,
            self.bytes_out,
            final_model.as_deref(),
            None,
        );
    }
}

pub async fn stream_forward(
    upstream_response: reqwest::Response,
    sse_acc: Option<SSEUsageAccumulator>,
    request_audit: CompressionAudit,
    path: String,
    method: String,
    request_model: Option<String>,
    bytes_in: usize,
    started_perf: Instant,
    audit_logger: Arc<AuditLogger>,
    cost_tracker: Arc<CostTracker>,
) -> Response {
    let status = upstream_response.status();
    let headers = upstream_response.headers().clone();
    let request_id = headers.get("request-id").and_then(|v| v.to_str().ok()).map(|s| s.to_string());

    let mut response_builder = Response::builder().status(status);
    for (name, val) in headers.iter() {
        let name_str = name.as_str().to_lowercase();
        if !crate::server::auth::HOP_BY_HOP_HEADERS.contains(&name_str.as_str())
            && !crate::server::auth::RESPONSE_STRIPPED_HEADERS.contains(&name_str.as_str())
        {
            response_builder = response_builder.header(name, val);
        }
    }

    // Set accept-encoding to identity for standard chunk decoding
    response_builder = response_builder.header("accept-encoding", "identity");

    let stream = upstream_response.bytes_stream();
    let logging_stream = LoggingStream {
        inner_stream: stream,
        sse_acc,
        request_audit,
        path,
        method,
        request_model,
        bytes_in,
        bytes_out: 0,
        started_perf,
        audit_logger,
        cost_tracker,
        status_code: status.as_u16(),
        request_id,
        logged: false,
    };

    let body = Body::from_stream(logging_stream);
    response_builder.body(body).unwrap_or_default()
}
