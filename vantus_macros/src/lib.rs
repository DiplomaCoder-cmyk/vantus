use proc_macro::TokenStream;
use quote::{ToTokens, quote};
use syn::{
    Attribute, FnArg, ImplItem, ImplItemFn, Item, ItemImpl, LitStr, Pat, PatIdent, Type,
    parse_macro_input,
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
    item
}

#[proc_macro_attribute]
pub fn post(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}

#[proc_macro_attribute]
pub fn put(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
}

#[proc_macro_attribute]
pub fn delete(_attr: TokenStream, item: TokenStream) -> TokenStream {
    item
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
    let route_methods = collect_route_methods(&item_impl)?;
    strip_route_attributes(&mut item_impl);

    let receiver_bound = if route_methods.iter().any(|route| route.uses_receiver) {
        Some(quote! {
            where #self_ty: ::core::clone::Clone + Send + Sync + 'static
        })
    } else {
        None
    };

    let forwarded_configure_routes = forward_method(&item_impl, "configure_routes");
    let forwarded_configure_routes_call = call_method(&item_impl, "configure_routes");
    let configure_routes = if route_methods.is_empty() {
        forwarded_configure_routes
    } else {
        let registrations = route_methods.iter().map(|route| {
            let method = &route.method;
            let path = &route.path;
            let method_name = &route.fn_name;
            let extract_stmts = &route.extract_stmts;
            let call_args = &route.call_args;

            let invocation = if route.uses_receiver {
                quote! {
                    let controller = self.clone();
                    let handler = ::vantus::__private::Handler::new(move |ctx: ::vantus::__private::RequestContext| {
                        let controller = controller.clone();
                        async move {
                            #(#extract_stmts)*
                            let result = controller.#method_name(#(#call_args),*);
                            ::vantus::__private::IntoHandlerResult::into_handler_result(result)
                        }
                    });
                }
            } else {
                quote! {
                    let handler = ::vantus::__private::Handler::new(move |ctx: ::vantus::__private::RequestContext| {
                        async move {
                            #(#extract_stmts)*
                            let result = <#self_ty>::#method_name(#(#call_args),*);
                            ::vantus::__private::IntoHandlerResult::into_handler_result(result)
                        }
                    });
                }
            };

            quote! {
                {
                    #invocation
                    routes.add_route(::vantus::__private::RouteDefinition::new(#method, #path, handler))?;
                }
            }
        });

        Some(quote! {
            fn configure_routes(&self, routes: &mut dyn ::vantus::__private::RouteRegistrar) -> Result<(), ::vantus::FrameworkError> {
                #forwarded_configure_routes_call
                #(#registrations)*
                Ok(())
            }
        })
    };

    let configure_services = forward_method(&item_impl, "configure_services");
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
            #configure_services
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

fn call_method(item_impl: &ItemImpl, method_name: &str) -> Option<proc_macro2::TokenStream> {
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
        <#self_ty>::#name(self, #(#params),*)?;
    })
}

struct RouteMethod {
    method: proc_macro2::TokenStream,
    path: LitStr,
    fn_name: syn::Ident,
    extract_stmts: Vec<proc_macro2::TokenStream>,
    call_args: Vec<proc_macro2::TokenStream>,
    uses_receiver: bool,
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
    let (http_method, path) = match find_route_attr(&method.attrs) {
        Some(Ok(result)) => result,
        Some(Err(error)) => return Some(Err(error)),
        None => return None,
    };

    let uses_receiver = method
        .sig
        .inputs
        .first()
        .map(|arg| matches!(arg, FnArg::Receiver(_)))
        .unwrap_or(false);

    let mut extract_stmts = Vec::new();
    let mut call_args = Vec::new();

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

        extract_stmts.push(match extraction_for(ident, pat_ty.ty.as_ref()) {
            Ok(stmt) => stmt,
            Err(error) => return Some(Err(error)),
        });
        call_args.push(quote! { #ident });
    }

    Some(Ok(RouteMethod {
        method: http_method,
        path,
        fn_name: method.sig.ident.clone(),
        extract_stmts,
        call_args,
        uses_receiver,
    }))
}

fn find_route_attr(
    attrs: &[Attribute],
) -> Option<Result<(proc_macro2::TokenStream, LitStr), syn::Error>> {
    for attr in attrs {
        let Some(ident) = attr.path().segments.last().map(|segment| &segment.ident) else {
            continue;
        };

        let method = match ident.to_string().as_str() {
            "get" => quote! { ::vantus::Method::Get },
            "post" => quote! { ::vantus::Method::Post },
            "put" => quote! { ::vantus::Method::Put },
            "delete" => quote! { ::vantus::Method::Delete },
            _ => continue,
        };

        return Some(attr.parse_args::<LitStr>().map(|path| (method, path)));
    }

    None
}

fn strip_route_attributes(item_impl: &mut ItemImpl) {
    for item in &mut item_impl.items {
        if let ImplItem::Fn(method) = item {
            method.attrs.retain(|attr| {
                !matches!(
                    attr.path().segments.last().map(|segment| segment.ident.to_string()),
                    Some(name) if matches!(name.as_str(), "get" | "post" | "put" | "delete")
                )
            });
        }
    }
}

fn extraction_for(ident: &syn::Ident, ty: &Type) -> Result<proc_macro2::TokenStream, syn::Error> {
    let _ = last_type_segment(ty)
        .ok_or_else(|| syn::Error::new_spanned(ty, "unsupported handler parameter type"))?;

    Ok(quote! {
        let #ident: #ty = ::vantus::__private::NamedFromRequest::from_request_named(&ctx, stringify!(#ident))?;
    })
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
