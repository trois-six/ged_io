//! Benchmarks for writing GEDCOM: the fixtures and a generated dataset
//! (individuals with notes long enough to be continued, families, sources),
//! each in the version it declares.

use criterion::{criterion_group, criterion_main, BenchmarkId, Criterion, Throughput};
use ged_io::{types::GedcomData, GedcomBuilder, GedcomWriter};
use std::fmt::Write as _;
use std::fs;
use std::hint::black_box;

/// A fictitious 5.5.1 dataset of `n` individuals, `n / 2` families and
/// `n / 10` sources.
fn generated(n: usize) -> String {
    let mut s = String::from("0 HEAD\n1 GEDC\n2 VERS 5.5.1\n2 FORM LINEAGE-LINKED\n1 CHAR UTF-8\n");
    for i in 0..n {
        let _ = write!(
            s,
            "0 @I{i}@ INDI\n1 NAME Given{i} /Family{}/\n1 SEX {}\n1 BIRT\n2 DATE {} JAN {}\n2 PLAC Sampletown, Region {}\n1 FAMS @F{}@\n1 NOTE {}\n2 CONT second line with an @ sign\n",
            i % 97,
            if i % 2 == 0 { "M" } else { "F" },
            1 + i % 28,
            1800 + i % 200,
            i % 13,
            i / 2,
            "word ".repeat(10 + i % 60).trim_end(),
        );
    }
    for f in 0..n / 2 {
        let _ = write!(s, "0 @F{f}@ FAM\n1 HUSB @I{}@\n1 WIFE @I{}@\n1 MARR\n2 DATE 1850\n2 SOUR @S{}@\n3 PAGE p. {f}\n", 2 * f, 2 * f + 1, f % (n / 10).max(1));
    }
    for i in 0..(n / 10).max(1) {
        let _ = write!(
            s,
            "0 @S{i}@ SOUR\n1 TITL Parish register {i}\n1 AUTH Sample archive\n"
        );
    }
    s.push_str("0 TRLR\n");
    s
}

fn bench_write(c: &mut Criterion) {
    let mut group = c.benchmark_group("write_to_string");
    let mut inputs: Vec<(String, String)> = [
        ("simple", "tests/fixtures/simple.ged"),
        ("sample", "tests/fixtures/sample.ged"),
        ("maximal551", "tests/fixtures/conformance/maximal551.ged"),
        ("washington", "tests/fixtures/washington.ged"),
    ]
    .iter()
    .filter_map(|(name, path)| Some(((*name).to_string(), fs::read_to_string(path).ok()?)))
    .collect();
    inputs.push(("generated-20k".to_string(), generated(20_000)));
    for (name, text) in inputs {
        let data: GedcomData = GedcomBuilder::new().build_from_str(&text).unwrap();
        let written = GedcomWriter::new().write_to_string(&data).unwrap();
        group.throughput(Throughput::Bytes(written.len() as u64));
        group.bench_with_input(BenchmarkId::from_parameter(&name), &data, |b, data| {
            b.iter(|| {
                GedcomWriter::new()
                    .write_to_string(black_box(data))
                    .unwrap()
            });
        });
    }
    group.finish();
}

criterion_group!(benches, bench_write);
criterion_main!(benches);
