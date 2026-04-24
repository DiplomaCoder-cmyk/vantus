use criterion::{BenchmarkId, Criterion, Throughput, black_box, criterion_group, criterion_main};
use vantus::{Method, Response};

const ROUTE_COUNTS: [usize; 3] = [50, 500, 5_000];

fn build_router(route_count: usize) -> vantus::__private::Router {
    let mut router = vantus::__private::Router::new();
    let handler = vantus::__private::Handler::new(|_| async move { Ok(Response::text("ok")) });
    for index in 0..route_count {
        let path = format!("/users/{index}/orders/{{order_id}}");
        vantus::__private::RouteRegistrar::add_route(
            &mut router,
            vantus::__private::RouteDefinition::new(Method::Get, path, handler.clone()),
        )
        .unwrap();
    }

    router
}

fn bench_dynamic_hits(c: &mut Criterion) {
    let mut group = c.benchmark_group("router_dynamic_hit");
    group.throughput(Throughput::Elements(1));

    for route_count in ROUTE_COUNTS {
        let router = build_router(route_count);
        group.bench_with_input(
            BenchmarkId::from_parameter(route_count),
            &router,
            |b, router| {
                b.iter(|| black_box(router.route(&Method::Get, "/users/42/orders/99")));
            },
        );
    }

    group.finish();
}

fn bench_normalized_hits(c: &mut Criterion) {
    let mut group = c.benchmark_group("router_normalized_hit");
    group.throughput(Throughput::Elements(1));

    for route_count in ROUTE_COUNTS {
        let router = build_router(route_count);
        group.bench_with_input(
            BenchmarkId::from_parameter(route_count),
            &router,
            |b, router| {
                b.iter(|| black_box(router.route(&Method::Get, "/users//42/orders/99/")));
            },
        );
    }

    group.finish();
}

fn bench_method_not_allowed(c: &mut Criterion) {
    let mut group = c.benchmark_group("router_method_not_allowed");
    group.throughput(Throughput::Elements(1));

    for route_count in ROUTE_COUNTS {
        let router = build_router(route_count);
        group.bench_with_input(
            BenchmarkId::from_parameter(route_count),
            &router,
            |b, router| {
                b.iter(|| black_box(router.resolve(&Method::Post, "/users/42/orders/99")));
            },
        );
    }

    group.finish();
}

fn bench_not_found(c: &mut Criterion) {
    let mut group = c.benchmark_group("router_not_found");
    group.throughput(Throughput::Elements(1));

    for route_count in ROUTE_COUNTS {
        let router = build_router(route_count);
        group.bench_with_input(
            BenchmarkId::from_parameter(route_count),
            &router,
            |b, router| {
                b.iter(|| black_box(router.resolve(&Method::Get, "/users/99999/orders")));
            },
        );
    }

    group.finish();
}

criterion_group!(
    benches,
    bench_dynamic_hits,
    bench_normalized_hits,
    bench_method_not_allowed,
    bench_not_found
);
criterion_main!(benches);
