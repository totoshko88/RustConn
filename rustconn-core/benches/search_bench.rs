//! Benchmarks for search performance
//!
//! Run with: `cargo bench -p rustconn-core`

#![allow(clippy::cast_possible_truncation)]
#![allow(clippy::cast_possible_wrap)]
#![allow(clippy::semicolon_if_nothing_returned)]
#![allow(clippy::explicit_iter_loop)]

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion};
use rustconn_core::models::{Connection, ConnectionGroup, ProtocolConfig, SshConfig};
use rustconn_core::search::{SearchEngine, SearchQuery};
use std::hint::black_box;
use uuid::Uuid;

/// Creates test connections for benchmarking
fn create_test_connections(count: usize) -> Vec<Connection> {
    (0..count)
        .map(|i| {
            Connection::new(
                format!("server-{i:04}"),
                format!("host-{i}.example.com"),
                22,
                ProtocolConfig::Ssh(SshConfig::default()),
            )
            .with_tags(vec![
                format!("env:{}", if i % 2 == 0 { "prod" } else { "dev" }),
                format!("region:{}", ["us-east", "us-west", "eu-west"][i % 3]),
            ])
        })
        .collect()
}

/// Creates test groups for benchmarking
fn create_test_groups(count: usize) -> Vec<ConnectionGroup> {
    (0..count)
        .map(|i| ConnectionGroup {
            id: Uuid::new_v4(),
            name: format!("group-{i}"),
            parent_id: None,
            sort_order: i as i32,
            expanded: true,
            created_at: chrono::Utc::now(),
        })
        .collect()
}

fn bench_search_scaling(c: &mut Criterion) {
    let mut group = c.benchmark_group("search_scaling");

    for size in [100, 500, 1000, 2000].iter() {
        let connections = create_test_connections(*size);
        let groups = create_test_groups(size / 10);
        let engine = SearchEngine::new();
        let query = SearchQuery::with_text("server");

        group.bench_with_input(BenchmarkId::new("connections", size), size, |b, _| {
            b.iter(|| {
                engine.search(
                    black_box(&query),
                    black_box(&connections),
                    black_box(&groups),
                )
            })
        });
    }

    group.finish();
}

fn bench_fuzzy_score(c: &mut Criterion) {
    let mut group = c.benchmark_group("fuzzy_score");
    let engine = SearchEngine::new();

    group.bench_function("exact_match", |b| {
        b.iter(|| engine.fuzzy_score(black_box("server"), black_box("server")))
    });

    group.bench_function("prefix_match", |b| {
        b.iter(|| engine.fuzzy_score(black_box("serv"), black_box("server-production-01")))
    });

    group.bench_function("fuzzy_match", |b| {
        b.iter(|| engine.fuzzy_score(black_box("srv"), black_box("server-production-01")))
    });

    group.bench_function("no_match", |b| {
        b.iter(|| engine.fuzzy_score(black_box("xyz"), black_box("server-production-01")))
    });

    group.finish();
}

fn bench_query_parsing(c: &mut Criterion) {
    let mut group = c.benchmark_group("query_parsing");

    group.bench_function("simple", |b| {
        b.iter(|| SearchEngine::parse_query(black_box("server")))
    });

    group.bench_function("with_filters", |b| {
        b.iter(|| {
            SearchEngine::parse_query(black_box(
                "server protocol:ssh tag:production group:servers",
            ))
        })
    });

    group.bench_function("complex", |b| {
        b.iter(|| {
            SearchEngine::parse_query(black_box(
                "web server protocol:ssh tag:prod tag:critical group:production prop:notes",
            ))
        })
    });

    group.finish();
}

fn bench_search_with_filters(c: &mut Criterion) {
    let mut group = c.benchmark_group("search_with_filters");
    let connections = create_test_connections(500);
    let groups = create_test_groups(20);
    let engine = SearchEngine::new();

    group.bench_function("no_filter", |b| {
        let query = SearchQuery::with_text("server");
        b.iter(|| {
            engine.search(
                black_box(&query),
                black_box(&connections),
                black_box(&groups),
            )
        })
    });

    group.bench_function("protocol_filter", |b| {
        let query = SearchEngine::parse_query("server protocol:ssh").unwrap();
        b.iter(|| {
            engine.search(
                black_box(&query),
                black_box(&connections),
                black_box(&groups),
            )
        })
    });

    group.bench_function("tag_filter", |b| {
        let query = SearchEngine::parse_query("server tag:prod").unwrap();
        b.iter(|| {
            engine.search(
                black_box(&query),
                black_box(&connections),
                black_box(&groups),
            )
        })
    });

    group.bench_function("multiple_filters", |b| {
        let query = SearchEngine::parse_query("server protocol:ssh tag:prod").unwrap();
        b.iter(|| {
            engine.search(
                black_box(&query),
                black_box(&connections),
                black_box(&groups),
            )
        })
    });

    group.finish();
}

criterion_group!(
    benches,
    bench_search_scaling,
    bench_fuzzy_score,
    bench_query_parsing,
    bench_search_with_filters,
);

criterion_main!(benches);
