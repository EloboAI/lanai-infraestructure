use actix_web::{
    body::{BoxBody, MessageBody},
    dev::{forward_ready, Service, ServiceRequest, ServiceResponse, Transform},
    Error, HttpMessage, HttpResponse,
};
use futures_util::future::LocalBoxFuture;
use std::future::{ready, Ready};
use std::sync::Arc;

/// Request size limiting middleware
pub struct RequestSizeLimitMiddleware {
    pub max_size: usize,
}

impl<S, B> Transform<S, ServiceRequest> for RequestSizeLimitMiddleware
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    S: 'static,
    B: MessageBody + 'static,
{
    type Response = ServiceResponse<BoxBody>;
    type Error = Error;
    type InitError = ();
    type Transform = RequestSizeLimitMiddlewareService<S>;
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ready(Ok(RequestSizeLimitMiddlewareService {
            service: Arc::new(service),
            max_size: self.max_size,
        }))
    }
}

pub struct RequestSizeLimitMiddlewareService<S> {
    service: Arc<S>,
    max_size: usize,
}

impl<S, B> Service<ServiceRequest> for RequestSizeLimitMiddlewareService<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error>,
    S::Future: 'static,
    S: 'static,
    B: MessageBody + 'static,
{
    type Response = ServiceResponse<BoxBody>;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    forward_ready!(service);

    fn call(&self, req: ServiceRequest) -> Self::Future {
        let service = Arc::clone(&self.service);
        let max_size = self.max_size;

        Box::pin(async move {
            // Check Content-Length header
            if let Some(content_length) = req.headers().get("content-length") {
                if let Ok(length_str) = content_length.to_str() {
                    if let Ok(length) = length_str.parse::<usize>() {
                        if length > max_size {
                            let response = HttpResponse::PayloadTooLarge()
                                .json(serde_json::json!({"error": format!("Request size {} exceeds maximum allowed size {}", length, max_size)}));
                            return Ok(req.into_response(response));
                        }
                    }
                }
            }

            service.call(req).await.map(|res| res.map_body(|_, body| body.boxed()))
        })
    }
}
