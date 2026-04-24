use proc_macro::TokenStream;
use quote::{ToTokens, quote};
use syn::{
    Attribute, FnArg, GenericArgument, ImplItem, ImplItemFn, Item, ItemImpl, LitStr, Pat, PatIdent,
    PathArguments, Type, TypePath, parse::Parse, parse::ParseStream, parse_macro_input,
    parse_quote,
};

#[proc_macro_attribute]
pub fn controller(_attr: TokenStream, item: TokenStream) -> TokenStream {
    expand_impl_macro(item, false)
}

#[proc_macro_attribute]
pub fn module(_attr: TokenStream, item: TokenStream) -> TokenStream {
    expand_impl_macro(item, true)
}

#[proc_macro_attribute]
pub fn get(_attr: TokenStream, item: TokenStream) -> TokenStream {
    reject_bare_route_macro("#[get]", item)
}

#[proc_macro_attribute]
pub fn post(_attr: TokenStream, item: TokenStream) -> TokenStream {
    reject_bare_route_macro("#[post]", item)
}

#[proc_macro_attribute]
pub fn put(_attr: TokenStream, item: TokenStream) -> TokenStream {
    reject_bare_route_macro("#[put]", item)
}

#[proc_macro_attribute]
pub fn delete(_attr: TokenStream, item: TokenStream) -> TokenStream {
    reject_bare_route_macro("#[delete]", item)
}

#[proc_macro_attribute]
pub fn patch(_attr: TokenStream, item: TokenStream) -> TokenStream {
    reject_bare_route_macro("#[patch]", item)
}

#[proc_macro_attribute]
pub fn head(_attr: TokenStream, item: TokenStream) -> TokenStream {
    reject_bare_route_macro("#[head]", item)
}

#[proc_macro_attribute]
pub fn options(_attr: TokenStream, item: TokenStream) -> TokenStream {
    reject_bare_route_macro("#[options]", item)
}

#[proc_macro_attribute]
pub fn middleware(attr: TokenStream, item: TokenStream) -> TokenStream {
    let parsed_item = match syn::parse::<Item>(item.clone()) {
        Ok(item) => item,
        Err(error) => return error.to_compile_error().into(),
    };

    if let Item::Impl(mut item_impl) = parsed_item {
        let include_runtime_hooks = item_impl
            .attrs
            .iter()
            .any(|attr| is_macro_attr(attr, &["module"]));
        let has_framework_macro = item_impl
            .attrs
            .iter()
            .any(|attr| is_macro_attr(attr, &["controller", "module"]));
        if has_framework_macro {
            // If `#[middleware(...)]` expands before `#[controller]` / `#[module]`, we still
            // want impl-level middleware to participate in the same route expansion path.
            let attr_tokens = proc_macro2::TokenStream::from(attr);
            let middleware_attr: Attribute = parse_quote!(#[middleware(#attr_tokens)]);
            item_impl
                .attrs
                .retain(|attr| !is_macro_attr(attr, &["controller", "module"]));
            item_impl.attrs.insert(0, middleware_attr);
            return match expand_impl(item_impl, include_runtime_hooks) {
                Ok(tokens) => tokens.into(),
                Err(error) => error.to_compile_error().into(),
            };
        }
    }

    reject_bare_route_macro("#[middleware(...)]", item)
}

fn expand_impl_macro(item: TokenStream, include_runtime_hooks: bool) -> TokenStream {
    let input = parse_macro_input!(item as Item);
    let Item::Impl(item_impl) = input else {
        return syn::Error::new_spanned(input, "attribute can only be used on impl blocks")
            .to_compile_error()
            .into();
    };

    match expand_impl(item_impl, include_runtime_hooks) {
        Ok(tokens) => tokens.into(),
        Err(error) => error.to_compile_error().into(),
    }
}

fn expand_impl(
    mut item_impl: ItemImpl,
    include_runtime_hooks: bool,
) -> Result<proc_macro2::TokenStream, syn::Error> {
    let self_ty = item_impl.self_ty.clone();
    let impl_middlewares = parse_middleware_attrs(
        &item_impl.attrs,
        "#[middleware(...)] must be used on a #[controller] or #[module] impl block or a route method",
    )?;
    let route_methods = collect_route_methods(&item_impl)?;
    strip_macro_attributes(&mut item_impl);

    let receiver_bound = if route_methods.iter().any(|route| route.uses_receiver) {
        Some(quote! {
            where #self_ty: Send + Sync + 'static
        })
    } else {
        None
    };

    let has_receiver_routes = route_methods.iter().any(|route| route.uses_receiver);
    let forwarded_configure_routes = forward_method(&item_impl, "configure_routes");
    let forwarded_configure_routes_call =
        call_method_on(&item_impl, "configure_routes", quote! { self });
    let forwarded_configure_routes_arc_call =
        call_method_on(&item_impl, "configure_routes", quote! { self.as_ref() });
    let configure_routes = if route_methods.is_empty() {
        forwarded_configure_routes
    } else {
        let registrations = route_methods.iter().map(|route| {
            let method = &route.method;
            let path = &route.path;
            let method_name = &route.fn_name;
            let extract_stmts = &route.extract_stmts;
            let call_args = &route.call_args;
            let contract = &route.contract;
            // Middleware order is encoded directly in the generated route definition:
            // impl-level attributes first, then route-level attributes, both in source order.
            let middleware = middleware_construction_tokens(
                impl_middlewares
                    .iter()
                    .chain(route.middlewares.iter()),
            );
            let response_conversion = if route.returns_result {
                quote! {
                    match result {
                        Ok(value) => ::vantus::__private::IntoResponse::into_response(value),
                        Err(error) => ::vantus::__private::IntoResponse::into_response(error),
                    }
                }
            } else {
                quote! {
                    ::vantus::__private::IntoResponse::into_response(result)
                }
            };
            let await_tokens = if route.is_async {
                quote! { .await }
            } else {
                quote! {}
            };

            let invocation = if route.uses_receiver {
                quote! {
                    let controller = ::std::sync::Arc::clone(&self);
                    let handler = ::vantus::__private::Handler::new(move |ctx: ::vantus::__private::RequestContext| {
                        let controller = ::std::sync::Arc::clone(&controller);
                        async move {
                            #(#extract_stmts)*
                            let result = controller.#method_name(#(#call_args),*) #await_tokens;
                            #response_conversion
                        }
                    });
                }
            } else {
                quote! {
                    let handler = ::vantus::__private::Handler::new(move |ctx: ::vantus::__private::RequestContext| {
                        async move {
                            #(#extract_stmts)*
                            let result = <#self_ty>::#method_name(#(#call_args),*) #await_tokens;
                            #response_conversion
                        }
                    });
                }
            };

            quote! {
                {
                    #invocation
                    // The runtime already knows how to combine global/module middleware with
                    // route-local middleware, so the macro only needs to attach the per-route
                    // vector here.
                    let definition = ::vantus::__private::RouteDefinition::new(#method, #path, handler)
                        .with_middleware(#middleware)
                        .with_contract(#contract);
                    routes.add_route(definition)?;
                }
            }
        });

        Some(if has_receiver_routes {
            quote! {
                fn configure_routes(&self, routes: &mut dyn ::vantus::__private::RouteRegistrar) -> Result<(), ::vantus::FrameworkError> {
                    #forwarded_configure_routes_call
                    let _ = routes;
                    Err(::vantus::FrameworkError::Startup {
                        context: "receiver-based macro routes must be registered through HostBuilder::module or RouteGroup::module".to_string(),
                    })
                }

                fn configure_routes_arc(
                    self: ::std::sync::Arc<Self>,
                    routes: &mut dyn ::vantus::__private::RouteRegistrar,
                ) -> Result<(), ::vantus::FrameworkError> {
                    #forwarded_configure_routes_arc_call
                    #(#registrations)*
                    Ok(())
                }
            }
        } else {
            quote! {
                fn configure_routes(&self, routes: &mut dyn ::vantus::__private::RouteRegistrar) -> Result<(), ::vantus::FrameworkError> {
                    #forwarded_configure_routes_call
                    #(#registrations)*
                    Ok(())
                }
            }
        })
    };

    let configure_middleware = forward_method(&item_impl, "configure_middleware");

    let runtime_impl = if include_runtime_hooks {
        let on_start = forward_async_method(&item_impl, "on_start");
        let on_stop = forward_async_method(&item_impl, "on_stop");
        Some(quote! {
            #[::vantus::__private::async_trait]
            impl ::vantus::RuntimeModule for #self_ty
            #receiver_bound
            {
                #on_start
                #on_stop
            }
        })
    } else {
        None
    };

    Ok(quote! {
        #item_impl

        impl ::vantus::Module for #self_ty
        #receiver_bound
        {
            #configure_middleware
            #configure_routes
        }

        #runtime_impl
    })
}

fn forward_method(item_impl: &ItemImpl, method_name: &str) -> Option<proc_macro2::TokenStream> {
    let self_ty = &item_impl.self_ty;
    let method = item_impl.items.iter().find_map(|item| match item {
        ImplItem::Fn(method) if method.sig.ident == method_name => Some(method),
        _ => None,
    })?;

    let name = &method.sig.ident;
    let params = method
        .sig
        .inputs
        .iter()
        .skip(1)
        .map(|arg| match arg {
            FnArg::Typed(pat) => pat.pat.to_token_stream(),
            FnArg::Receiver(_) => quote! { self },
        })
        .collect::<Vec<_>>();
    let signature = &method.sig.inputs;
    let output = &method.sig.output;

    Some(quote! {
        fn #name(#signature) #output {
            <#self_ty>::#name(self, #(#params),*)
        }
    })
}

fn forward_async_method(
    item_impl: &ItemImpl,
    method_name: &str,
) -> Option<proc_macro2::TokenStream> {
    let self_ty = &item_impl.self_ty;
    let method = item_impl.items.iter().find_map(|item| match item {
        ImplItem::Fn(method) if method.sig.ident == method_name => Some(method),
        _ => None,
    })?;

    let name = &method.sig.ident;
    let params = method
        .sig
        .inputs
        .iter()
        .skip(1)
        .map(|arg| match arg {
            FnArg::Typed(pat) => pat.pat.to_token_stream(),
            FnArg::Receiver(_) => quote! { self },
        })
        .collect::<Vec<_>>();
    let signature = &method.sig.inputs;
    let output = &method.sig.output;

    Some(quote! {
        async fn #name(#signature) #output {
            <#self_ty>::#name(self, #(#params),*).await
        }
    })
}

fn call_method_on(
    item_impl: &ItemImpl,
    method_name: &str,
    receiver: proc_macro2::TokenStream,
) -> Option<proc_macro2::TokenStream> {
    let self_ty = &item_impl.self_ty;
    let method = item_impl.items.iter().find_map(|item| match item {
        ImplItem::Fn(method) if method.sig.ident == method_name => Some(method),
        _ => None,
    })?;

    let name = &method.sig.ident;
    let params = method
        .sig
        .inputs
        .iter()
        .skip(1)
        .map(|arg| match arg {
            FnArg::Typed(pat) => pat.pat.to_token_stream(),
            FnArg::Receiver(_) => quote! { self },
        })
        .collect::<Vec<_>>();

    Some(quote! {
        <#self_ty>::#name(#receiver, #(#params),*)?;
    })
}

struct RouteMethod {
    method: proc_macro2::TokenStream,
    path: LitStr,
    fn_name: syn::Ident,
    extract_stmts: Vec<proc_macro2::TokenStream>,
    call_args: Vec<proc_macro2::TokenStream>,
    contract: proc_macro2::TokenStream,
    middlewares: Vec<TypePath>,
    uses_receiver: bool,
    is_async: bool,
    returns_result: bool,
}

fn collect_route_methods(item_impl: &ItemImpl) -> Result<Vec<RouteMethod>, syn::Error> {
    let mut routes = Vec::new();
    let mut errors: Option<syn::Error> = None;

    for item in &item_impl.items {
        let ImplItem::Fn(method) = item else {
            continue;
        };

        match parse_route_method(method) {
            Some(Ok(route)) => routes.push(route),
            Some(Err(error)) => {
                if let Some(existing) = &mut errors {
                    existing.combine(error);
                } else {
                    errors = Some(error);
                }
            }
            None => {}
        }
    }

    if let Some(error) = errors {
        Err(error)
    } else {
        Ok(routes)
    }
}

fn parse_route_method(method: &ImplItemFn) -> Option<Result<RouteMethod, syn::Error>> {
    let middlewares = match parse_middleware_attrs(
        &method.attrs,
        "#[middleware(...)] must be used on a route method or the enclosing #[controller]/#[module] impl block",
    ) {
        Ok(middlewares) => middlewares,
        Err(error) => return Some(Err(error)),
    };
    let (http_method, method_kind, path) = match find_route_attr(&method.attrs) {
        Some(Ok(result)) => result,
        Some(Err(error)) => return Some(Err(error)),
        None => {
            if let Some(attr) = find_middleware_attr(&method.attrs) {
                return Some(Err(syn::Error::new_spanned(
                    attr,
                    "#[middleware(...)] on methods requires a route attribute such as #[get(...)]",
                )));
            }
            return None;
        }
    };
    if let Err(error) = validate_route_path(&path) {
        return Some(Err(error));
    }
    let route_path_params = route_path_parameters(&path);

    let uses_receiver = method
        .sig
        .inputs
        .first()
        .map(|arg| matches!(arg, FnArg::Receiver(_)))
        .unwrap_or(false);

    let mut extract_stmts = Vec::new();
    let mut call_args = Vec::new();
    let mut body_extractors = Vec::new();

    for input in &method.sig.inputs {
        let FnArg::Typed(pat_ty) = input else {
            continue;
        };
        let Pat::Ident(PatIdent { ident, .. }) = pat_ty.pat.as_ref() else {
            return Some(Err(syn::Error::new_spanned(
                &pat_ty.pat,
                "handler parameters must be simple identifiers",
            )));
        };

        let extraction = match extraction_for(ident, pat_ty.ty.as_ref()) {
            Ok(stmt) => stmt,
            Err(error) => return Some(Err(error)),
        };
        if let Some(path_param) = &extraction.path_param {
            if !route_path_params.contains(path_param) {
                return Some(Err(syn::Error::new_spanned(
                    &pat_ty.ty,
                    format!(
                        "path parameter `{}` is not present in route template `{}`",
                        path_param,
                        path.value()
                    ),
                )));
            }
        }
        if let Some(body_kind) = extraction.body_kind {
            body_extractors.push(body_kind);
        }
        extract_stmts.push(extraction.stmt);
        call_args.push(quote! { #ident });
    }

    if body_extractors.len() > 1 {
        return Some(Err(syn::Error::new_spanned(
            &method.sig,
            "handlers may declare at most one body extractor",
        )));
    }

    if !body_extractors.is_empty()
        && matches!(
            method_kind,
            HttpMethodKind::Get | HttpMethodKind::Head | HttpMethodKind::Options
        )
    {
        return Some(Err(syn::Error::new_spanned(
            &method.sig,
            "GET, HEAD, and OPTIONS handlers may not declare body extractors",
        )));
    }

    let contract = route_contract_tokens(body_extractors.first().copied());

    Some(Ok(RouteMethod {
        method: http_method,
        path,
        fn_name: method.sig.ident.clone(),
        extract_stmts,
        call_args,
        contract,
        middlewares,
        uses_receiver,
        is_async: method.sig.asyncness.is_some(),
        returns_result: return_type_is_result(&method.sig.output),
    }))
}

fn find_route_attr(
    attrs: &[Attribute],
) -> Option<Result<(proc_macro2::TokenStream, HttpMethodKind, LitStr), syn::Error>> {
    for attr in attrs {
        let Some(ident) = attr.path().segments.last().map(|segment| &segment.ident) else {
            continue;
        };

        let (method, method_kind) = match ident.to_string().as_str() {
            "get" => (quote! { ::vantus::Method::Get }, HttpMethodKind::Get),
            "post" => (quote! { ::vantus::Method::Post }, HttpMethodKind::Post),
            "put" => (quote! { ::vantus::Method::Put }, HttpMethodKind::Put),
            "delete" => (quote! { ::vantus::Method::Delete }, HttpMethodKind::Delete),
            "patch" => (quote! { ::vantus::Method::Patch }, HttpMethodKind::Patch),
            "head" => (quote! { ::vantus::Method::Head }, HttpMethodKind::Head),
            "options" => (
                quote! { ::vantus::Method::Options },
                HttpMethodKind::Options,
            ),
            _ => continue,
        };

        return Some(
            attr.parse_args::<LitStr>()
                .map(|path| (method, method_kind, path)),
        );
    }

    None
}

fn strip_macro_attributes(item_impl: &mut ItemImpl) {
    item_impl
        .attrs
        .retain(|attr| !is_macro_attr(attr, &["middleware"]));
    for item in &mut item_impl.items {
        if let ImplItem::Fn(method) = item {
            method.attrs.retain(|attr| {
                !is_macro_attr(
                    attr,
                    &[
                        "get",
                        "post",
                        "put",
                        "delete",
                        "patch",
                        "head",
                        "options",
                        "middleware",
                    ],
                )
            });
        }
    }
}

fn parse_middleware_attrs(
    attrs: &[Attribute],
    usage_message: &str,
) -> Result<Vec<TypePath>, syn::Error> {
    let mut middlewares = Vec::new();
    let mut errors: Option<syn::Error> = None;

    for attr in attrs
        .iter()
        .filter(|attr| is_macro_attr(attr, &["middleware"]))
    {
        match attr.parse_args::<MiddlewareTypeArg>() {
            Ok(arg) => middlewares.push(arg.ty),
            Err(error) => {
                let error = syn::Error::new_spanned(attr, format!("{usage_message}: {error}"));
                if let Some(existing) = &mut errors {
                    existing.combine(error);
                } else {
                    errors = Some(error);
                }
            }
        }
    }

    if let Some(error) = errors {
        Err(error)
    } else {
        Ok(middlewares)
    }
}

fn middleware_construction_tokens<'a>(
    middlewares: impl Iterator<Item = &'a TypePath>,
) -> proc_macro2::TokenStream {
    let middleware_entries = middlewares.map(|middleware| {
        quote! {
            // This intentionally relies on `Default` so unsupported middleware types fail
            // with normal compile-time trait-bound diagnostics.
            ::std::sync::Arc::new(
                <#middleware as ::std::default::Default>::default()
            ) as ::std::sync::Arc<dyn ::vantus::Middleware>
        }
    });

    quote! {
        vec![#(#middleware_entries),*]
    }
}

fn find_middleware_attr(attrs: &[Attribute]) -> Option<&Attribute> {
    attrs
        .iter()
        .find(|attr| is_macro_attr(attr, &["middleware"]))
}

fn is_macro_attr(attr: &Attribute, names: &[&str]) -> bool {
    matches!(
        attr.path().segments.last().map(|segment| segment.ident.to_string()),
        Some(name) if names.iter().any(|candidate| name == *candidate)
    )
}

fn extraction_for(ident: &syn::Ident, ty: &Type) -> Result<ExtractionPlan, syn::Error> {
    let outer = last_type_segment(ty)
        .ok_or_else(|| syn::Error::new_spanned(ty, "unsupported handler parameter type"))?;

    if outer == "Option" {
        let inner = option_inner_type(ty).ok_or_else(|| {
            syn::Error::new_spanned(ty, "Option<T> requires a concrete inner type")
        })?;
        let inner_outer = last_type_segment(inner).ok_or_else(|| {
            syn::Error::new_spanned(inner, "unsupported optional handler parameter type")
        })?;
        return match inner_outer.as_str() {
            "Query" | "Header" => Ok(ExtractionPlan {
                stmt: quote! {
                    let #ident: #ty = ::vantus::__private::NamedOptionalFromRequest::from_request_optional_named(&ctx, stringify!(#ident))?;
                },
                body_kind: None,
                path_param: None,
            }),
            "RequestState" | "IdentityState" => Ok(ExtractionPlan {
                stmt: quote! {
                    let #ident: #ty = ::vantus::__private::OptionalFromRequest::from_request_optional(&ctx)?;
                },
                body_kind: None,
                path_param: None,
            }),
            _ => Err(syn::Error::new_spanned(
                ty,
                "optional handler parameters are only supported for Query<T>, Header<T>, RequestState<T>, and IdentityState<T>",
            )),
        };
    }

    match outer.as_str() {
        "Path" => Ok(ExtractionPlan {
            stmt: quote! {
                let #ident: #ty = ::vantus::__private::NamedFromRequest::from_request_named(&ctx, stringify!(#ident))?;
            },
            body_kind: None,
            path_param: Some(ident.to_string()),
        }),
        "Query" | "Header" => Ok(ExtractionPlan {
            stmt: quote! {
                let #ident: #ty = ::vantus::__private::NamedFromRequest::from_request_named(&ctx, stringify!(#ident))?;
            },
            body_kind: None,
            path_param: None,
        }),
        "RequestContext" | "QueryMap" | "RequestState" | "IdentityState" => Ok(ExtractionPlan {
            stmt: quote! {
                let #ident: #ty = ::vantus::FromRequest::from_request(&ctx)?;
            },
            body_kind: None,
            path_param: None,
        }),
        "BodyBytes" => Ok(ExtractionPlan {
            stmt: quote! {
                let #ident: #ty = ::vantus::FromRequest::from_request(&ctx)?;
            },
            body_kind: Some(BodyExtractorKind::Bytes),
            path_param: None,
        }),
        "TextBody" => Ok(ExtractionPlan {
            stmt: quote! {
                let #ident: #ty = ::vantus::FromRequest::from_request(&ctx)?;
            },
            body_kind: Some(BodyExtractorKind::Text),
            path_param: None,
        }),
        "JsonBody" => Ok(ExtractionPlan {
            stmt: quote! {
                let #ident: #ty = ::vantus::FromRequest::from_request(&ctx)?;
            },
            body_kind: Some(BodyExtractorKind::Json),
            path_param: None,
        }),
        _ => Err(syn::Error::new_spanned(
            ty,
            "unsupported handler parameter type; use request-derived extractors such as RequestContext, Path<T>, Query<T>, Header<T>, QueryMap, BodyBytes, TextBody, JsonBody<T>, RequestState<T>, or IdentityState<T>",
        )),
    }
}

struct ExtractionPlan {
    stmt: proc_macro2::TokenStream,
    body_kind: Option<BodyExtractorKind>,
    path_param: Option<String>,
}

#[derive(Clone, Copy)]
enum BodyExtractorKind {
    Bytes,
    Text,
    Json,
}

#[derive(Clone, Copy)]
enum HttpMethodKind {
    Get,
    Post,
    Put,
    Delete,
    Patch,
    Head,
    Options,
}

fn option_inner_type(ty: &Type) -> Option<&Type> {
    let Type::Path(type_path) = ty else {
        return None;
    };
    let segment = type_path.path.segments.last()?;
    if segment.ident != "Option" {
        return None;
    }
    let PathArguments::AngleBracketed(arguments) = &segment.arguments else {
        return None;
    };
    arguments.args.iter().find_map(|arg| match arg {
        GenericArgument::Type(ty) => Some(ty),
        _ => None,
    })
}

fn route_contract_tokens(body_kind: Option<BodyExtractorKind>) -> proc_macro2::TokenStream {
    match body_kind {
        Some(BodyExtractorKind::Bytes) => quote! {
            ::vantus::__private::RouteContract::new(::vantus::__private::RequestBodyKind::Bytes)
        },
        Some(BodyExtractorKind::Text) => quote! {
            ::vantus::__private::RouteContract::new(::vantus::__private::RequestBodyKind::Text)
        },
        Some(BodyExtractorKind::Json) => quote! {
            ::vantus::__private::RouteContract::new(::vantus::__private::RequestBodyKind::Json)
        },
        None => quote! {
            ::vantus::__private::RouteContract::default()
        },
    }
}

fn route_path_parameters(path: &LitStr) -> std::collections::HashSet<String> {
    path.value()
        .split('/')
        .filter(|segment| segment.starts_with('{') && segment.ends_with('}'))
        .map(|segment| segment[1..segment.len() - 1].to_string())
        .collect()
}

fn last_type_segment(ty: &Type) -> Option<String> {
    let Type::Path(type_path) = ty else {
        return None;
    };
    type_path
        .path
        .segments
        .last()
        .map(|segment| segment.ident.to_string())
}

fn return_type_is_result(output: &syn::ReturnType) -> bool {
    let syn::ReturnType::Type(_, ty) = output else {
        return false;
    };
    matches!(last_type_segment(ty.as_ref()).as_deref(), Some("Result"))
}

fn validate_route_path(path: &LitStr) -> Result<(), syn::Error> {
    let value = path.value();
    if !value.starts_with('/') || value.contains('\0') {
        return Err(syn::Error::new_spanned(
            path,
            "route paths must start with '/'",
        ));
    }

    let mut params = std::collections::HashSet::new();
    for segment in value.split('/').filter(|segment| !segment.is_empty()) {
        if segment.starts_with('{') || segment.ends_with('}') {
            if !(segment.starts_with('{') && segment.ends_with('}')) {
                return Err(syn::Error::new_spanned(
                    path,
                    "route parameter segments must use balanced braces",
                ));
            }
            let name = &segment[1..segment.len() - 1];
            let is_valid = !name.is_empty()
                && name
                    .chars()
                    .next()
                    .map(|ch| ch == '_' || ch.is_ascii_alphabetic())
                    .unwrap_or(false)
                && name
                    .chars()
                    .all(|ch| ch == '_' || ch.is_ascii_alphanumeric());
            if !is_valid {
                return Err(syn::Error::new_spanned(
                    path,
                    format!("invalid route parameter name `{name}`"),
                ));
            }
            if !params.insert(name.to_string()) {
                return Err(syn::Error::new_spanned(
                    path,
                    format!("duplicate route parameter name `{name}`"),
                ));
            }
        }
    }

    Ok(())
}

fn reject_bare_route_macro(name: &str, item: TokenStream) -> TokenStream {
    let item_clone = item.clone();
    let parsed = syn::parse::<Item>(item_clone);
    let allowed_placeholder = matches!(parsed, Ok(Item::Fn(_)) | Ok(Item::Impl(_)));
    if allowed_placeholder {
        let tokens = proc_macro2::TokenStream::from(item);
        quote! {
            #tokens
            compile_error!(concat!(#name, " must be used inside a #[controller] or #[module] impl block"));
        }
        .into()
    } else {
        item
    }
}

struct MiddlewareTypeArg {
    ty: TypePath,
}

impl Parse for MiddlewareTypeArg {
    fn parse(input: ParseStream<'_>) -> syn::Result<Self> {
        if input.is_empty() {
            return Err(input.error("expected a middleware type path like AuthGuard"));
        }

        let ty = input.parse::<TypePath>()?;
        if !input.is_empty() {
            return Err(input.error("expected exactly one middleware type path"));
        }

        Ok(Self { ty })
    }
}
