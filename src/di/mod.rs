use bytes::Bytes;
use serde::{Serialize, de::DeserializeOwned};
use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;

use crate::core::errors::FrameworkError;
use crate::core::http::Response;
use crate::routing::{Identity, RequestContext};

/// Extracts a named path segment from the matched route.
#[derive(Clone, Debug)]
pub struct Path<T>(T);

impl<T> Path<T> {
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> AsRef<T> for Path<T> {
    fn as_ref(&self) -> &T {
        &self.0
    }
}

/// Extracts a named query-string value.
#[derive(Clone, Debug)]
pub struct Query<T>(T);

impl<T> Query<T> {
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> AsRef<T> for Query<T> {
    fn as_ref(&self) -> &T {
        &self.0
    }
}

/// Extracts and parses a named request header.
#[derive(Clone, Debug)]
pub struct Header<T>(T);

impl<T> Header<T> {
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> AsRef<T> for Header<T> {
    fn as_ref(&self) -> &T {
        &self.0
    }
}

/// Provides access to the full query map.
#[derive(Clone, Debug)]
pub struct QueryMap(pub HashMap<String, Vec<String>>);

impl QueryMap {
    pub fn get(&self, key: &str) -> Option<&str> {
        self.0
            .get(key)
            .and_then(|values| values.first())
            .map(String::as_str)
    }
}

/// Zero-copy request body bytes.
#[derive(Clone, Debug)]
pub struct BodyBytes(pub Bytes);

impl BodyBytes {
    pub fn as_slice(&self) -> &[u8] {
        self.0.as_ref()
    }
}

/// UTF-8 text body extractor.
#[derive(Clone, Debug)]
pub struct TextBody(pub String);

impl TextBody {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// JSON request body extractor.
#[derive(Clone, Debug)]
pub struct JsonBody<T>(pub T);

impl<T> JsonBody<T> {
    pub fn into_inner(self) -> T {
        self.0
    }
}

impl<T> AsRef<T> for JsonBody<T> {
    fn as_ref(&self) -> &T {
        &self.0
    }
}

/// Typed request-local state inserted by middleware.
///
/// Middleware writes values with `RequestContext::insert_state(value)`,
/// then handlers can request `RequestState<T>` as a parameter.
#[derive(Clone, Debug)]
pub struct RequestState<T>(std::sync::Arc<T>);

impl<T> RequestState<T> {
    pub fn into_inner(self) -> std::sync::Arc<T> {
        self.0
    }
}

impl<T> AsRef<T> for RequestState<T> {
    fn as_ref(&self) -> &T {
        &self.0
    }
}

/// Typed request identity inserted by middleware.
///
/// Middleware writes values with `RequestContext::insert_identity(value)`,
/// then handlers can request `IdentityState<T>` as a parameter.
#[derive(Clone, Debug)]
pub struct IdentityState<T>(std::sync::Arc<T>);

impl<T> IdentityState<T> {
    pub fn into_inner(self) -> std::sync::Arc<T> {
        self.0
    }
}

impl<T> AsRef<T> for IdentityState<T> {
    fn as_ref(&self) -> &T {
        &self.0
    }
}

#[derive(Debug)]
pub enum ExtractorError {
    Missing(String),
    ParseFailed(String),
}

impl fmt::Display for ExtractorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ExtractorError::Missing(field) => write!(f, "missing field: {field}"),
            ExtractorError::ParseFailed(field) => write!(f, "failed to parse field: {field}"),
        }
    }
}

impl std::error::Error for ExtractorError {}

/// Generic extraction contract for handler parameters.
pub trait FromRequest: Sized {
    fn from_request(ctx: &RequestContext) -> Result<Self, FrameworkError>;
}

/// Named extraction contract used by `Path<T>` and `Query<T>`.
pub trait NamedFromRequest: Sized {
    fn from_request_named(ctx: &RequestContext, name: &str) -> Result<Self, FrameworkError>;
}

/// Optional extraction contract used for safe request-derived inputs.
pub trait OptionalFromRequest: Sized {
    fn from_request_optional(ctx: &RequestContext) -> Result<Option<Self>, FrameworkError>;
}

/// Optional named extraction contract used by `Option<Query<T>>` and `Option<Header<T>>`.
pub trait NamedOptionalFromRequest: Sized {
    fn from_request_optional_named(
        ctx: &RequestContext,
        name: &str,
    ) -> Result<Option<Self>, FrameworkError>;
}

impl<T> NamedFromRequest for T
where
    T: FromRequest,
{
    fn from_request_named(ctx: &RequestContext, _name: &str) -> Result<Self, FrameworkError> {
        T::from_request(ctx)
    }
}

impl<T> NamedOptionalFromRequest for T
where
    T: OptionalFromRequest,
{
    fn from_request_optional_named(
        ctx: &RequestContext,
        _name: &str,
    ) -> Result<Option<Self>, FrameworkError> {
        T::from_request_optional(ctx)
    }
}

impl FromRequest for RequestContext {
    fn from_request(ctx: &RequestContext) -> Result<Self, FrameworkError> {
        Ok(ctx.clone())
    }
}

impl<T> FromRequest for RequestState<T>
where
    T: Send + Sync + 'static,
{
    fn from_request(ctx: &RequestContext) -> Result<Self, FrameworkError> {
        ctx.require_state::<T>().map(RequestState)
    }
}

impl<T> OptionalFromRequest for RequestState<T>
where
    T: Send + Sync + 'static,
{
    fn from_request_optional(ctx: &RequestContext) -> Result<Option<Self>, FrameworkError> {
        Ok(ctx.state::<T>().map(RequestState))
    }
}

impl<T> FromRequest for IdentityState<T>
where
    T: Identity,
{
    fn from_request(ctx: &RequestContext) -> Result<Self, FrameworkError> {
        ctx.require_identity::<T>().map(IdentityState)
    }
}

impl<T> OptionalFromRequest for IdentityState<T>
where
    T: Identity,
{
    fn from_request_optional(ctx: &RequestContext) -> Result<Option<Self>, FrameworkError> {
        Ok(ctx.identity::<T>().map(IdentityState))
    }
}

impl<T> FromRequest for Option<T>
where
    T: OptionalFromRequest,
{
    fn from_request(ctx: &RequestContext) -> Result<Self, FrameworkError> {
        T::from_request_optional(ctx)
    }
}

impl FromRequest for QueryMap {
    fn from_request(ctx: &RequestContext) -> Result<Self, FrameworkError> {
        Ok(QueryMap(ctx.request().query_params.clone()))
    }
}

impl FromRequest for BodyBytes {
    fn from_request(ctx: &RequestContext) -> Result<Self, FrameworkError> {
        Ok(BodyBytes(ctx.request().body.clone()))
    }
}

impl FromRequest for TextBody {
    fn from_request(ctx: &RequestContext) -> Result<Self, FrameworkError> {
        std::str::from_utf8(ctx.request().body.as_ref())
            .map(|body| TextBody(body.to_string()))
            .map_err(|_| ExtractorError::ParseFailed("body".to_string()).into())
    }
}

impl<T> FromRequest for JsonBody<T>
where
    T: DeserializeOwned,
{
    fn from_request(ctx: &RequestContext) -> Result<Self, FrameworkError> {
        serde_json::from_slice::<T>(ctx.request().body.as_ref())
            .map(JsonBody)
            .map_err(|_| ExtractorError::ParseFailed("body".to_string()).into())
    }
}

impl<T> NamedFromRequest for Path<T>
where
    T: FromStr,
{
    fn from_request_named(ctx: &RequestContext, name: &str) -> Result<Self, FrameworkError> {
        let raw = ctx
            .path_params()
            .get(name)
            .ok_or_else(|| ExtractorError::Missing(name.to_string()))?;
        raw.parse::<T>()
            .map(Path)
            .map_err(|_| ExtractorError::ParseFailed(name.to_string()).into())
    }
}

impl<T> NamedFromRequest for Query<T>
where
    T: FromStr,
{
    fn from_request_named(ctx: &RequestContext, name: &str) -> Result<Self, FrameworkError> {
        let raw = ctx
            .request()
            .query_params
            .get(name)
            .and_then(|values| values.first())
            .ok_or_else(|| ExtractorError::Missing(name.to_string()))?;
        raw.parse::<T>()
            .map(Query)
            .map_err(|_| ExtractorError::ParseFailed(name.to_string()).into())
    }
}

impl<T> NamedOptionalFromRequest for Query<T>
where
    T: FromStr,
{
    fn from_request_optional_named(
        ctx: &RequestContext,
        name: &str,
    ) -> Result<Option<Self>, FrameworkError> {
        let Some(raw) = ctx
            .request()
            .query_params
            .get(name)
            .and_then(|values| values.first())
        else {
            return Ok(None);
        };

        raw.parse::<T>()
            .map(Query)
            .map(Some)
            .map_err(|_| ExtractorError::ParseFailed(name.to_string()).into())
    }
}

impl<T> NamedFromRequest for Header<T>
where
    T: FromStr,
{
    fn from_request_named(ctx: &RequestContext, name: &str) -> Result<Self, FrameworkError> {
        let header_name = normalize_header_name(name);
        let values = header_values(ctx, &header_name)?;
        let raw = values
            .first()
            .ok_or_else(|| ExtractorError::Missing(header_name.clone()))?;

        raw.parse::<T>()
            .map(Header)
            .map_err(|_| ExtractorError::ParseFailed(header_name).into())
    }
}

impl<T> NamedOptionalFromRequest for Header<T>
where
    T: FromStr,
{
    fn from_request_optional_named(
        ctx: &RequestContext,
        name: &str,
    ) -> Result<Option<Self>, FrameworkError> {
        let header_name = normalize_header_name(name);
        let values = header_values(ctx, &header_name)?;
        let Some(raw) = values.first() else {
            return Ok(None);
        };

        raw.parse::<T>()
            .map(Header)
            .map(Some)
            .map_err(|_| ExtractorError::ParseFailed(header_name).into())
    }
}

/// Converts typed handler return values into the framework response model.
pub trait IntoResponse {
    fn into_response(self) -> Result<Response, FrameworkError>;
}

impl IntoResponse for Response {
    fn into_response(self) -> Result<Response, FrameworkError> {
        Ok(self)
    }
}

impl IntoResponse for FrameworkError {
    fn into_response(self) -> Result<Response, FrameworkError> {
        Ok(self.to_response())
    }
}

impl IntoResponse for crate::core::errors::HttpError {
    fn into_response(self) -> Result<Response, FrameworkError> {
        Ok(self.to_response())
    }
}

impl<T> IntoResponse for T
where
    T: Serialize,
{
    fn into_response(self) -> Result<Response, FrameworkError> {
        Response::json_serialized(&self)
            .map_err(|_| FrameworkError::internal("response serialization failed"))
    }
}

/// Compatibility shim for the older macro return-conversion name.
pub trait IntoHandlerResult {
    fn into_handler_result(self) -> Result<Response, FrameworkError>;
}

impl<T> IntoHandlerResult for T
where
    T: IntoResponse,
{
    fn into_handler_result(self) -> Result<Response, FrameworkError> {
        self.into_response()
    }
}

fn normalize_header_name(name: &str) -> String {
    name.to_ascii_lowercase().replace('_', "-")
}

fn header_values(ctx: &RequestContext, name: &str) -> Result<Vec<String>, FrameworkError> {
    let values = ctx
        .request()
        .headers
        .get_all(name)
        .iter()
        .map(|value| {
            value
                .to_str()
                .map(str::to_string)
                .map_err(|_| ExtractorError::ParseFailed(name.to_string()).into())
        })
        .collect::<Result<Vec<_>, FrameworkError>>()?;

    if values.len() > 1 {
        return Err(ExtractorError::ParseFailed(name.to_string()).into());
    }

    Ok(values)
}
