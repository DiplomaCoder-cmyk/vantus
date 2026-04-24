use std::collections::HashMap;

use matchit::{InsertError, Router as MatchItRouter};

use crate::core::errors::FrameworkError;
use crate::core::http::Method;

use super::path::normalize_request_path;
use super::route::{Route, RouteContract};
use super::{Handler, RouteDefinition};

pub trait RouteRegistrar {
    fn add_route(&mut self, definition: RouteDefinition) -> Result<(), FrameworkError>;
}

struct MethodRouteIndex {
    matcher: MatchItRouter<usize>,
    routes: Vec<Route>,
}

impl Default for MethodRouteIndex {
    fn default() -> Self {
        Self {
            matcher: MatchItRouter::new(),
            routes: Vec::new(),
        }
    }
}

/// Internal route index used by `HostBuilder` and macro-generated modules.
pub struct Router {
    indexes_by_method: HashMap<Method, MethodRouteIndex>,
}

impl Router {
    pub fn new() -> Self {
        Self {
            indexes_by_method: HashMap::new(),
        }
    }

    /// Adds a fully constructed route definition to the router.
    pub fn add_definition(&mut self, definition: RouteDefinition) -> Result<(), FrameworkError> {
        let route = Route::from_definition(definition)?;
        let method = route.method.clone();
        let method_index = self.indexes_by_method.entry(method.clone()).or_default();
        let route_id = method_index.routes.len();

        method_index
            .matcher
            .insert(route.path.as_str(), route_id)
            .map_err(|error| map_insert_error(error, &method, route.path.as_str()))?;
        method_index.routes.push(route);
        Ok(())
    }

    /// Finds the best matching route for an incoming method/path pair.
    pub fn route(&self, method: &Method, path: &str) -> Option<RouteMatch> {
        match self.resolve(method, path) {
            RouteResolution::Matched(route) => Some(route),
            RouteResolution::MethodNotAllowed { .. } | RouteResolution::NotFound => None,
        }
    }

    pub fn resolve(&self, method: &Method, path: &str) -> RouteResolution {
        let Ok(path) = normalize_request_path(path) else {
            return RouteResolution::NotFound;
        };

        let mut allow = Vec::new();

        if let Some(method_index) = self.indexes_by_method.get(method) {
            if let Ok(matched) = method_index.matcher.at(path.as_str()) {
                if let Some(route) = method_index.routes.get(*matched.value) {
                    let path_params = matched
                        .params
                        .iter()
                        .map(|(key, value)| (key.to_string(), value.to_string()))
                        .collect();

                    return RouteResolution::Matched(RouteMatch {
                        handler: route.handler.clone(),
                        middleware: route.middleware.clone(),
                        path_params,
                        route_pattern: route.route_pattern.clone(),
                        contract: route.contract,
                    });
                }
            }
        }

        for (candidate_method, index) in &self.indexes_by_method {
            if candidate_method == method {
                continue;
            }

            if index.matcher.at(path.as_str()).is_ok() {
                allow.push(candidate_method.clone());
            }
        }

        if allow.is_empty() {
            RouteResolution::NotFound
        } else {
            allow.sort_by_key(method_sort_key);
            allow.dedup();
            RouteResolution::MethodNotAllowed { allow }
        }
    }

    pub fn format_allow_header(methods: &[Method]) -> String {
        let mut unique = methods.to_vec();
        unique.sort_by_key(method_sort_key);
        unique.dedup();
        unique
            .into_iter()
            .map(|method| method.to_string())
            .collect::<Vec<_>>()
            .join(", ")
    }
}

impl Default for Router {
    fn default() -> Self {
        Self::new()
    }
}

impl RouteRegistrar for Router {
    fn add_route(&mut self, definition: RouteDefinition) -> Result<(), FrameworkError> {
        self.add_definition(definition)
    }
}

#[derive(Clone)]
pub struct RouteMatch {
    pub handler: Handler,
    pub middleware: Vec<std::sync::Arc<dyn crate::middleware::Middleware>>,
    pub path_params: HashMap<String, String>,
    pub route_pattern: std::sync::Arc<str>,
    pub contract: RouteContract,
}

pub enum RouteResolution {
    Matched(RouteMatch),
    MethodNotAllowed { allow: Vec<Method> },
    NotFound,
}

fn map_insert_error(error: InsertError, method: &Method, new_path: &str) -> FrameworkError {
    match error {
        InsertError::Conflict { with } => FrameworkError::RouteConflict {
            method: method.to_string(),
            existing_path: with,
            new_path: new_path.to_string(),
        },
        InsertError::InvalidParamSegment => FrameworkError::Startup {
            context: "invalid route path segment: only one parameter is allowed per path segment"
                .to_string(),
        },
        InsertError::InvalidParam => FrameworkError::Startup {
            context: format!("invalid route path: {new_path}"),
        },
        InsertError::InvalidCatchAll => FrameworkError::Startup {
            context: format!("invalid route path: {new_path}"),
        },
        _ => FrameworkError::Startup {
            context: format!("invalid route path: {new_path}"),
        },
    }
}

fn method_sort_key(method: &Method) -> u8 {
    match method {
        Method::Get => 0,
        Method::Head => 1,
        Method::Post => 2,
        Method::Put => 3,
        Method::Delete => 4,
        Method::Patch => 5,
        Method::Options => 6,
        Method::Other(_) => 7,
    }
}

#[cfg(test)]
mod tests {
    use crate::core::http::{Method, Response};

    use super::super::{Handler, RouteDefinition};
    use super::Router;

    #[test]
    fn matches_static_routes() {
        let mut router = Router::new();
        router
            .add_definition(RouteDefinition::new(
                Method::Get,
                "/health",
                Handler::new(|_| async move { Ok(Response::text("ok")) }),
            ))
            .unwrap();

        assert!(router.route(&Method::Get, "/health").is_some());
    }

    #[test]
    fn matches_parameter_routes() {
        let mut router = Router::new();
        router
            .add_definition(RouteDefinition::new(
                Method::Get,
                "/users/{id}",
                Handler::new(|_| async move { Ok(Response::text("ok")) }),
            ))
            .unwrap();

        let matched = router.route(&Method::Get, "/users/42").unwrap();
        assert_eq!(
            matched.path_params.get("id").map(String::as_str),
            Some("42")
        );
    }

    #[test]
    fn matches_root_routes() {
        let mut router = Router::new();
        router
            .add_definition(RouteDefinition::new(
                Method::Get,
                "/",
                Handler::new(|_| async move { Ok(Response::text("ok")) }),
            ))
            .unwrap();

        assert!(router.route(&Method::Get, "/").is_some());
    }

    #[test]
    fn isolates_methods() {
        let mut router = Router::new();
        router
            .add_definition(RouteDefinition::new(
                Method::Post,
                "/users/{id}",
                Handler::new(|_| async move { Ok(Response::text("ok")) }),
            ))
            .unwrap();

        assert!(router.route(&Method::Get, "/users/42").is_none());
        assert!(router.route(&Method::Post, "/users/42").is_some());
    }

    #[test]
    fn reports_equivalent_parameter_conflicts() {
        let mut router = Router::new();
        router
            .add_definition(RouteDefinition::new(
                Method::Get,
                "/users/{id}",
                Handler::new(|_| async move { Ok(Response::text("ok")) }),
            ))
            .unwrap();

        let error = router
            .add_definition(RouteDefinition::new(
                Method::Get,
                "/users/{name}",
                Handler::new(|_| async move { Ok(Response::text("ok")) }),
            ))
            .unwrap_err();
        let message = error.to_string();

        assert!(message.contains("/users/{id}"));
        assert!(message.contains("/users/{name}"));
    }

    #[test]
    fn normalizes_lookup_paths_before_matching() {
        let mut router = Router::new();
        router
            .add_definition(RouteDefinition::new(
                Method::Get,
                "/users/{id}",
                Handler::new(|_| async move { Ok(Response::text("ok")) }),
            ))
            .unwrap();

        let matched = router.route(&Method::Get, "/users//42/").unwrap();
        assert_eq!(
            matched.path_params.get("id").map(String::as_str),
            Some("42")
        );
    }

    #[test]
    fn prefers_static_routes_over_dynamic_routes() {
        let mut router = Router::new();
        router
            .add_definition(RouteDefinition::new(
                Method::Get,
                "/users/{id}",
                Handler::new(|_| async move { Ok(Response::text("dynamic")) }),
            ))
            .unwrap();
        router
            .add_definition(RouteDefinition::new(
                Method::Get,
                "/users/me",
                Handler::new(|_| async move { Ok(Response::text("static")) }),
            ))
            .unwrap();

        let matched = router.route(&Method::Get, "/users/me").unwrap();
        assert!(matched.path_params.is_empty());
    }
}
