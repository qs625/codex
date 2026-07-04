use std::sync::Arc;

use transport_client::HttpTransport;
use transport_client::Request;
use transport_client::RequestBody;
use transport_client::Response;
use transport_client::StreamResponse;
use transport_client::TransportError;
use transport_client_types::RequestTelemetry;
use http::HeaderMap;
use http::Method;
use model_service_api::ApiError;
use model_service_api::Provider;
use model_service_api::SharedAuthProvider;
use serde_json::Value;

use crate::transport_telemetry::run_with_request_telemetry;

pub(crate) struct EndpointSession<T: HttpTransport> {
    transport: T,
    provider: Provider,
    auth: SharedAuthProvider,
    request_telemetry: Option<Arc<dyn RequestTelemetry>>,
}

impl<T: HttpTransport> EndpointSession<T> {
    pub(crate) fn new(transport: T, provider: Provider, auth: SharedAuthProvider) -> Self {
        Self {
            transport,
            provider,
            auth,
            request_telemetry: None,
        }
    }

    pub(crate) fn with_request_telemetry(
        mut self,
        request: Option<Arc<dyn RequestTelemetry>>,
    ) -> Self {
        self.request_telemetry = request;
        self
    }

    pub(crate) fn provider(&self) -> &Provider {
        &self.provider
    }

    fn make_request(
        &self,
        method: &Method,
        path: &str,
        extra_headers: &HeaderMap,
        body: Option<&Value>,
    ) -> Request {
        let mut request = self.provider.build_request(method.clone(), path);
        request.headers.extend(extra_headers.clone());
        if let Some(body) = body {
            request.body = Some(RequestBody::Json(body.clone()));
        }
        request
    }

    pub(crate) async fn execute(
        &self,
        method: Method,
        path: &str,
        extra_headers: HeaderMap,
        body: Option<Value>,
    ) -> Result<Response, ApiError> {
        self.execute_with(method, path, extra_headers, body, |_| {})
            .await
    }

    pub(crate) async fn execute_with<C>(
        &self,
        method: Method,
        path: &str,
        extra_headers: HeaderMap,
        body: Option<Value>,
        configure: C,
    ) -> Result<Response, ApiError>
    where
        C: Fn(&mut Request),
    {
        let make_request = || {
            let mut request = self.make_request(&method, path, &extra_headers, body.as_ref());
            configure(&mut request);
            request
        };

        run_with_request_telemetry(
            self.provider.retry.to_policy(),
            self.request_telemetry.clone(),
            make_request,
            |request| {
                let auth = self.auth.clone();
                let transport = &self.transport;
                async move {
                    let request = auth
                        .apply_auth(request)
                        .await
                        .map_err(TransportError::from)?;
                    transport.execute(request).await
                }
            },
        )
        .await
        .map_err(ApiError::from)
    }

    pub(crate) async fn stream_with<C>(
        &self,
        method: Method,
        path: &str,
        extra_headers: HeaderMap,
        body: Option<Value>,
        configure: C,
    ) -> Result<StreamResponse, ApiError>
    where
        C: Fn(&mut Request),
    {
        let make_request = || {
            let mut request = self.make_request(&method, path, &extra_headers, body.as_ref());
            configure(&mut request);
            request
        };

        run_with_request_telemetry(
            self.provider.retry.to_policy(),
            self.request_telemetry.clone(),
            make_request,
            |request| {
                let auth = self.auth.clone();
                let transport = &self.transport;
                async move {
                    let request = auth
                        .apply_auth(request)
                        .await
                        .map_err(TransportError::from)?;
                    transport.stream(request).await
                }
            },
        )
        .await
        .map_err(ApiError::from)
    }
}
