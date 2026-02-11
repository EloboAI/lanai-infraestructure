use actix_web::{
    dev::{Service, ServiceRequest, ServiceResponse, Transform},
    Error, FromRequest, HttpMessage, HttpRequest,
};
use futures_util::future::{ok, LocalBoxFuture, Ready};
use uuid::Uuid;
use std::rc::Rc;
use crate::middleware::auth_guard::Claims;

#[derive(Debug, Clone, Copy)]
pub struct TenantContext {
    pub org_id: Uuid,
}

impl FromRequest for TenantContext {
    type Error = actix_web::Error;
    type Future = Ready<Result<Self, Self::Error>>;

    fn from_request(req: &HttpRequest, _payload: &mut actix_web::dev::Payload) -> Self::Future {
        if let Some(ctx) = req.extensions().get::<TenantContext>() {
            return ok(*ctx);
        }
        // Fail if not found - ensuring security
        futures_util::future::err(actix_web::error::ErrorForbidden("Tenant context required"))
    }
}

pub struct TenantMiddleware;

impl<S, B> Transform<S, ServiceRequest> for TenantMiddleware
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type InitError = ();
    type Transform = TenantMiddlewareService<S>;
    type Future = Ready<Result<Self::Transform, Self::InitError>>;

    fn new_transform(&self, service: S) -> Self::Future {
        ok(TenantMiddlewareService {
            service: Rc::new(service),
        })
    }
}

pub struct TenantMiddlewareService<S> {
    service: Rc<S>,
}

impl<S, B> Service<ServiceRequest> for TenantMiddlewareService<S>
where
    S: Service<ServiceRequest, Response = ServiceResponse<B>, Error = Error> + 'static,
    S::Future: 'static,
    B: 'static,
{
    type Response = ServiceResponse<B>;
    type Error = Error;
    type Future = LocalBoxFuture<'static, Result<Self::Response, Self::Error>>;

    fn poll_ready(&self, ctx: &mut core::task::Context<'_>) -> core::task::Poll<Result<(), Self::Error>> {
        self.service.poll_ready(ctx)
    }

    fn call(&self, req: ServiceRequest) -> Self::Future {
        let service = self.service.clone();

        Box::pin(async move {
            let claims = req.extensions().get::<Claims>().cloned();
            let mut org_id_to_set = None;

            // 1. Try to get org_id from Claims (Secure Source)
            if let Some(ref c) = claims {
                if let Some(ref oid) = c.org_id {
                    // Token is Scoped! Use this.
                     if let Ok(uuid) = Uuid::parse_str(oid) {
                        org_id_to_set = Some(uuid);
                     }
                }
            } else {
                // 2. Fallback to Header ONLY if Claims are missing (Public Routes)
                if let Some(header_val) = req.headers().get("X-Organization-ID") {
                    if let Ok(header_str) = header_val.to_str() {
                        if let Ok(uuid) = Uuid::parse_str(header_str) {
                            org_id_to_set = Some(uuid);
                        }
                    }
                }
            }

            if let Some(oid) = org_id_to_set {
                 req.extensions_mut().insert(TenantContext { org_id: oid });
            }

            service.call(req).await
        })
    }
}
