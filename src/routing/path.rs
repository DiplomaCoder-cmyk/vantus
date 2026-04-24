use std::collections::HashSet;

use crate::core::errors::FrameworkError;

pub(crate) fn normalize_path_template(path: &str) -> Result<String, FrameworkError> {
    normalize_path(path, PathKind::RouteTemplate)
}

pub(crate) fn normalize_request_path(path: &str) -> Result<String, FrameworkError> {
    normalize_path(path, PathKind::Request)
}

fn normalize_path(path: &str, kind: PathKind) -> Result<String, FrameworkError> {
    if !path.starts_with('/') || path.contains('\0') || path.contains('\\') {
        return Err(invalid_path_error(kind, path));
    }

    let mut params = HashSet::new();
    let mut normalized_segments = Vec::new();

    for segment in path.split('/') {
        if segment.is_empty() {
            continue;
        }
        if segment == "." || segment == ".." {
            return Err(traversal_error(kind, path));
        }
        if matches!(kind, PathKind::RouteTemplate) {
            if segment.starts_with('{') || segment.ends_with('}') {
                validate_param_segment(segment, &mut params)?;
            } else if segment.contains('{') || segment.contains('}') {
                return Err(FrameworkError::startup(format!(
                    "invalid route path segment: {segment}"
                )));
            }
        }
        normalized_segments.push(segment.to_string());
    }

    if normalized_segments.is_empty() {
        Ok("/".to_string())
    } else {
        Ok(format!("/{}", normalized_segments.join("/")))
    }
}

fn invalid_path_error(kind: PathKind, path: &str) -> FrameworkError {
    match kind {
        PathKind::RouteTemplate => FrameworkError::startup(format!("invalid route path: {path}")),
        PathKind::Request => FrameworkError::startup(format!("invalid request path: {path}")),
    }
}

fn traversal_error(kind: PathKind, path: &str) -> FrameworkError {
    match kind {
        PathKind::RouteTemplate => {
            FrameworkError::startup(format!("route path contains traversal sequences: {path}"))
        }
        PathKind::Request => {
            FrameworkError::startup(format!("request path contains traversal sequences: {path}"))
        }
    }
}

#[derive(Clone, Copy)]
enum PathKind {
    RouteTemplate,
    Request,
}

fn validate_param_segment(
    segment: &str,
    params: &mut HashSet<String>,
) -> Result<(), FrameworkError> {
    if !(segment.starts_with('{') && segment.ends_with('}')) {
        return Err(FrameworkError::startup(format!(
            "invalid route path segment: {segment}"
        )));
    }

    let name = &segment[1..segment.len() - 1];
    if name.is_empty()
        || !name
            .chars()
            .all(|ch| ch == '_' || ch.is_ascii_alphanumeric())
        || !name
            .chars()
            .next()
            .map(|ch| ch == '_' || ch.is_ascii_alphabetic())
            .unwrap_or(false)
    {
        return Err(FrameworkError::startup(format!(
            "invalid route parameter name: {name}"
        )));
    }

    if !params.insert(name.to_string()) {
        return Err(FrameworkError::startup(format!(
            "duplicate route parameter name: {name}"
        )));
    }

    Ok(())
}

#[cfg(test)]
mod tests {
    use super::{normalize_path_template, normalize_request_path};

    #[test]
    fn normalizes_route_templates() {
        let normalized = normalize_path_template("//users//{id}//").unwrap();
        assert_eq!(normalized, "/users/{id}");
    }

    #[test]
    fn normalizes_request_paths() {
        let normalized = normalize_request_path("/users//42/").unwrap();
        assert_eq!(normalized, "/users/42");
    }

    #[test]
    fn rejects_traversal_sequences() {
        assert!(normalize_request_path("/users/../admin").is_err());
        assert!(normalize_path_template("/users/../{id}").is_err());
    }
}
