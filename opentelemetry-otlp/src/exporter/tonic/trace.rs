use core::fmt;

use futures_core::future::BoxFuture;
use opentelemetry::{otel_debug, trace::TraceError};
use opentelemetry_proto::tonic::collector::trace::v1::{
    trace_service_client::TraceServiceClient, ExportTraceServiceRequest,
};
use opentelemetry_sdk::trace::{ExportResult, SpanData, SpanExporter};
use tonic::{codegen::CompressionEncoding, service::Interceptor, transport::Channel, Request};

use opentelemetry_proto::transform::trace::tonic::group_spans_by_resource_and_scope;

use super::BoxInterceptor;

pub(crate) struct TonicTracesClient {
    inner: Option<ClientInner>,
    #[allow(dead_code)]
    // <allow dead> would be removed once we support set_resource for metrics.
    resource: opentelemetry_proto::transform::common::tonic::ResourceAttributesWithSchema,
}

struct ClientInner {
    client: TraceServiceClient<Channel>,
    interceptor: BoxInterceptor,
}

impl fmt::Debug for TonicTracesClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("TonicTracesClient")
    }
}

impl TonicTracesClient {
    pub(super) fn new(
        channel: Channel,
        interceptor: BoxInterceptor,
        compression: Option<CompressionEncoding>,
    ) -> Self {
        let mut client = TraceServiceClient::new(channel);
        if let Some(compression) = compression {
            client = client
                .send_compressed(compression)
                .accept_compressed(compression);
        }

        otel_debug!(name: "TonicsTracesClientBuilt");

        TonicTracesClient {
            inner: Some(ClientInner {
                client,
                interceptor,
            }),
            resource: Default::default(),
        }
    }
}

impl SpanExporter for TonicTracesClient {
    fn export(&mut self, batch: Vec<SpanData>) -> BoxFuture<'static, ExportResult> {
        let (mut client, metadata, extensions) = match &mut self.inner {
            Some(inner) => {
                let (m, e, _) = match inner.interceptor.call(Request::new(())) {
                    Ok(res) => res.into_parts(),
                    Err(e) => {
                        return Box::pin(std::future::ready(Err(TraceError::Other(Box::new(e)))))
                    }
                };
                (inner.client.clone(), m, e)
            }
            None => {
                return Box::pin(std::future::ready(Err(TraceError::Other(
                    "exporter is already shut down".into(),
                ))))
            }
        };

        let resource_spans = group_spans_by_resource_and_scope(batch, &self.resource);

        otel_debug!(name: "TonicsTracesClient.CallingExport");

        Box::pin(async move {
            client
                .export(Request::from_parts(
                    metadata,
                    extensions,
                    ExportTraceServiceRequest { resource_spans },
                ))
                .await
                .map_err(crate::Error::from)?;

            Ok(())
        })
    }

    fn shutdown(&mut self) {
        let _ = self.inner.take();
    }

    fn set_resource(&mut self, resource: &opentelemetry_sdk::Resource) {
        self.resource = resource.into();
    }
}
