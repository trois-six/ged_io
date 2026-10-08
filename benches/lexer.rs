//! The line lexer and lossless tree against the token-based tokenizer.
//!
//! Both read the same decoded text: the tokenizer to its end, the tree to a
//! complete arena (with continuations joined and `@@` unescaped), so the
//! comparison is conservative for the tree. `cargo bench --bench lexer`.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use ged_io::tokenizer::Tokenizer;
use ged_io::tree::{parse_tree, TreeReader};
use std::fmt::Write as _;
use std::fs;
use std::hint::black_box;

/// A fictitious 5.5.1 file of `n` individuals and `n / 2` families.
fn synthetic(n: usize) -> String {
    let mut s = String::from("0 HEAD\n1 GEDC\n2 VERS 5.5.1\n2 FORM LINEAGE-LINKED\n1 CHAR UTF-8\n");
    for i in 0..n {
        let _ = write!(
            s,
            "0 @I{i}@ INDI\n1 NAME Given{i} /Surname{}/\n2 GIVN Given{i}\n2 SURN Surname{}\n\
             1 SEX {}\n1 BIRT\n2 DATE {} JAN {}\n2 PLAC Town {}, Region, Country\n\
             1 NOTE A note about person {i}, long enough to be realistic\n2 CONT and continued @@ here.\n\
             1 FAMS @F{}@\n",
            i % 97,
            i % 97,
            if i % 2 == 0 { 'M' } else { 'F' },
            1 + i % 28,
            1800 + i % 200,
            i % 50,
            i / 2
        );
    }
    for f in 0..n / 2 {
        let _ = write!(
            s,
            "0 @F{f}@ FAM\n1 HUSB @I{}@\n1 WIFE @I{}@\n1 MARR\n2 DATE 1850\n",
            2 * f,
            2 * f + 1
        );
    }
    s.push_str("0 TRLR\n");
    s
}

fn tokenize_all(content: &str) {
    let mut tokenizer = Tokenizer::new(content.chars());
    while tokenizer.next_token().is_ok() && !tokenizer.done() {}
}

fn bench_lexer(c: &mut Criterion) {
    let mut inputs: Vec<(String, String)> = Vec::new();
    for (name, path) in [
        ("washington", "tests/fixtures/washington.ged"),
        ("maximal551", "tests/fixtures/conformance/maximal551.ged"),
    ] {
        if let Ok(content) = fs::read_to_string(path) {
            inputs.push((name.to_string(), content));
        }
    }
    inputs.push(("synthetic-20k".to_string(), synthetic(20_000)));

    let mut group = c.benchmark_group("lexer");
    for (name, content) in &inputs {
        group.throughput(Throughput::Bytes(content.len() as u64));
        group.bench_with_input(BenchmarkId::new("tokenizer", name), content, |b, c| {
            b.iter(|| tokenize_all(black_box(c)));
        });
        group.bench_with_input(BenchmarkId::new("tree", name), content, |b, c| {
            b.iter(|| parse_tree(black_box(c)));
        });
        group.bench_with_input(BenchmarkId::new("tree-stream", name), content, |b, c| {
            b.iter(|| {
                TreeReader::new(black_box(c.as_bytes()))
                    .map(|r| r.count())
                    .unwrap_or(0)
            });
        });
    }
    group.finish();
}

criterion_group!(benches, bench_lexer);
criterion_main!(benches);
