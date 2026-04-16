use std::collections::HashMap;
use std::fmt;
use std::str::FromStr;

use serde::de::DeserializeOwned;

use crate::app::{Config, Service};
use crate::config::FromConfiguration;
use crate::core::errors::FrameworkError;
use crate::core::http::Response;
use crate::routing::RequestContext;

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

#[derive(Clone, Debug)]
pub struct BodyBytes(pub Vec<u8>);

impl BodyBytes {
    pub fn as_slice(&self) -> &[u8] {
        &self.0
    }
}

#[derive(Clone, Debug)]
pub struct TextBody(pub String);

impl TextBody {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

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

pub trait FromRequest: Sized {
    fn from_request(ctx: &RequestContext) -> Result<Self, FrameworkError>;
}

pub trait NamedFromRequest: Sized {
    fn from_request_named(ctx: &RequestContext, name: &str) -> Result<Self, FrameworkError>;
}

impl<T> NamedFromRequest for T
where
    T: FromRequest,
{
    fn from_request_named(ctx: &RequestContext, _name: &str) -> Result<Self, FrameworkError> {
        T::from_request(ctx)
    }
}

impl FromRequest for RequestContext {
    fn from_request(ctx: &RequestContext) -> Result<Self, FrameworkError> {
        Ok(ctx.clone())
    }
}

impl<T> FromRequest for Service<T>
where
    T: Send + Sync + 'static,
{
    fn from_request(ctx: &RequestContext) -> Result<Self, FrameworkError> {
        ctx.service::<T>()
    }
}

impl<T> FromRequest for Config<T>
where
    T: Send + Sync + 'static,
{
    fn from_request(ctx: &RequestContext) -> Result<Self, FrameworkError> {
        ctx.config::<T>()
    }
}

impl<T> FromRequest for T
where
    T: FromConfiguration,
{
    fn from_request(ctx: &RequestContext) -> Result<Self, FrameworkError> {
        ctx.bind_config::<T>()
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
        String::from_utf8(ctx.request().body.clone())
            .map(TextBody)
            .map_err(|_| ExtractorError::ParseFailed("body".to_string()).into())
    }
}

impl<T> FromRequest for JsonBody<T>
where
    T: DeserializeOwned,
{
    fn from_request(ctx: &RequestContext) -> Result<Self, FrameworkError> {
        serde_json::from_slice::<T>(&ctx.request().body)
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

pub trait IntoHandlerResult {
    fn into_handler_result(self) -> Result<Response, FrameworkError>;
}

impl IntoHandlerResult for Response {
    fn into_handler_result(self) -> Result<Response, FrameworkError> {
        Ok(self)
    }
}

impl IntoHandlerResult for Result<Response, FrameworkError> {
    fn into_handler_result(self) -> Result<Response, FrameworkError> {
        self
    }
}
