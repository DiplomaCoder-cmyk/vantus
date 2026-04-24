use std::collections::HashSet;
use std::net::IpAddr;
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::Duration;

use vantus::GlobalRateLimiter;

#[test]
fn token_bucket_consumes_until_capacity_is_exhausted() {
    let limiter = GlobalRateLimiter::new(2, 1, Duration::from_secs(60));
    let ip: IpAddr = "203.0.113.10".parse().unwrap();

    assert!(limiter.check(ip));
    assert!(limiter.check(ip));
    assert!(!limiter.check(ip));
}

#[test]
fn token_bucket_refills_after_interval() {
    let limiter = GlobalRateLimiter::new(1, 1, Duration::from_millis(25));
    let ip: IpAddr = "203.0.113.20".parse().unwrap();

    assert!(limiter.check(ip));
    assert!(!limiter.check(ip));
    std::thread::sleep(Duration::from_millis(40));
    assert!(limiter.check(ip));
}

#[test]
fn token_bucket_tracks_ips_independently() {
    let limiter = GlobalRateLimiter::new(1, 1, Duration::from_secs(60));

    assert!(limiter.check("203.0.113.30".parse().unwrap()));
    assert!(limiter.check("198.51.100.40".parse().unwrap()));
    assert!(!limiter.check("203.0.113.30".parse().unwrap()));
}

#[test]
fn token_bucket_is_thread_safe_under_concurrency() {
    let limiter = Arc::new(GlobalRateLimiter::new(
        10_000,
        10_000,
        Duration::from_secs(60),
    ));
    let hits = Arc::new(Mutex::new(Vec::new()));

    thread::scope(|scope| {
        for idx in 0..8u8 {
            let limiter = Arc::clone(&limiter);
            let hits = Arc::clone(&hits);
            scope.spawn(move || {
                let ip: IpAddr = format!("203.0.113.{}", idx + 1).parse().unwrap();
                for _ in 0..64 {
                    if limiter.check(ip) {
                        hits.lock().unwrap().push(ip);
                    }
                }
            });
        }
    });

    let hits = hits.lock().unwrap();
    let unique = hits.iter().copied().collect::<HashSet<_>>();
    assert_eq!(unique.len(), 8);
    assert_eq!(hits.len(), 8 * 64);
}
