//! The typed model under construction (`ged_io::next`) against the lossless
//! tree it is read like, and the current model: reading and writing the
//! fixtures and a generated dataset. `cargo bench --bench model`.
//!
//! With `MODEL_RSS=<pipeline>` (`tree`, `next` or `current`), it instead
//! reads the file `MODEL_FILE` (a generated one of `MODEL_PEOPLE`
//! individuals, 330 000 by default, about 80 MB, when unset) once with that
//! pipeline and prints the input size, the time and the peak resident set
//! (Linux `VmHWM`): run it once per pipeline, each in its own process.

use criterion::{criterion_group, BenchmarkId, Criterion, Throughput};
use ged_io::tree::Tree;
use ged_io::{next, GedcomBuilder, GedcomWriter};
use std::alloc::{GlobalAlloc, Layout, System};
use std::fmt::Write as _;
use std::fs;
use std::hint::black_box;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Instant;

/// The system allocator, counting the bytes allocated and not freed.
struct Counting;

static LIVE: AtomicUsize = AtomicUsize::new(0);

// SAFETY: every call is forwarded to the system allocator unchanged.
unsafe impl GlobalAlloc for Counting {
    unsafe fn alloc(&self, layout: Layout) -> *mut u8 {
        LIVE.fetch_add(layout.size(), Ordering::Relaxed);
        // SAFETY: the caller's contract is the system allocator's.
        unsafe { System.alloc(layout) }
    }

    unsafe fn dealloc(&self, ptr: *mut u8, layout: Layout) {
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        // SAFETY: the caller's contract is the system allocator's.
        unsafe { System.dealloc(ptr, layout) }
    }

    unsafe fn realloc(&self, ptr: *mut u8, layout: Layout, new_size: usize) -> *mut u8 {
        LIVE.fetch_add(new_size, Ordering::Relaxed);
        LIVE.fetch_sub(layout.size(), Ordering::Relaxed);
        // SAFETY: the caller's contract is the system allocator's.
        unsafe { System.realloc(ptr, layout, new_size) }
    }
}

#[global_allocator]
static ALLOCATOR: Counting = Counting;

/// A fictitious 5.5.1 dataset of `n` individuals, `n / 2` families and
/// `n / 10` sources, with notes, places, dates and citations.
fn generated(n: usize) -> String {
    let mut s = String::from("0 HEAD\n1 GEDC\n2 VERS 5.5.1\n2 FORM LINEAGE-LINKED\n1 CHAR UTF-8\n");
    for i in 0..n {
        let _ = write!(
            s,
            "0 @I{i}@ INDI\n1 NAME Given{i} /Family{}/\n1 SEX {}\n1 BIRT\n2 DATE {} JAN {}\n2 PLAC Sampletown, Region {}\n2 SOUR @S{}@\n3 PAGE p. {i}\n1 FAMS @F{}@\n1 NOTE {}\n2 CONT second line with an @@ sign\n1 CHAN\n2 DATE 1 JAN 2000\n",
            i % 97,
            if i % 2 == 0 { "M" } else { "F" },
            1 + i % 28,
            1800 + i % 200,
            i % 13,
            i % (n / 10).max(1),
            i / 2,
            "word ".repeat(10 + i % 60).trim_end(),
        );
    }
    for f in 0..n / 2 {
        let _ = write!(
            s,
            "0 @F{f}@ FAM\n1 HUSB @I{}@\n1 WIFE @I{}@\n1 MARR\n2 DATE 1850\n2 SOUR @S{}@\n3 PAGE p. {f}\n",
            2 * f,
            2 * f + 1,
            f % (n / 10).max(1)
        );
    }
    for i in 0..(n / 10).max(1) {
        let _ = write!(
            s,
            "0 @S{i}@ SOUR\n1 TITL Parish register {i}\n1 AUTH Sample archive\n1 REPO @R1@\n2 CALN {i}\n"
        );
    }
    s.push_str("0 @R1@ REPO\n1 NAME Sample archive\n0 TRLR\n");
    s
}

fn inputs() -> Vec<(String, String)> {
    let mut inputs: Vec<(String, String)> = [
        ("maximal551", "tests/fixtures/conformance/maximal551.ged"),
        ("washington", "tests/fixtures/washington.ged"),
    ]
    .iter()
    .filter_map(|(name, path)| Some(((*name).to_string(), fs::read_to_string(path).ok()?)))
    .collect();
    inputs.push(("generated-20k".to_string(), generated(20_000)));
    inputs
}

fn bench_read(c: &mut Criterion) {
    let mut group = c.benchmark_group("model-read");
    for (name, text) in inputs() {
        group.throughput(Throughput::Bytes(text.len() as u64));
        group.bench_with_input(BenchmarkId::new("tree", &name), &text, |b, t| {
            b.iter(|| Tree::parse(black_box(t.clone())));
        });
        group.bench_with_input(BenchmarkId::new("next", &name), &text, |b, t| {
            b.iter(|| next::read_string(black_box(t.clone())));
        });
        group.bench_with_input(BenchmarkId::new("current", &name), &text, |b, t| {
            b.iter(|| GedcomBuilder::new().build_from_str(black_box(t)));
        });
    }
    group.finish();
}

fn bench_write(c: &mut Criterion) {
    let mut group = c.benchmark_group("model-write");
    for (name, text) in inputs() {
        let tree = Tree::parse(text.clone());
        let data = next::read_str(&text);
        let current = GedcomBuilder::new().build_from_str(&text).unwrap();
        let writer = GedcomWriter::new();
        let size = next::write_string(&data, &writer).unwrap().len();
        group.throughput(Throughput::Bytes(size as u64));
        group.bench_with_input(BenchmarkId::new("tree", &name), &tree, |b, t| {
            b.iter(|| {
                let mut out = Vec::with_capacity(size);
                writer.write_tree(&mut out, black_box(t)).unwrap();
                out
            });
        });
        group.bench_with_input(BenchmarkId::new("next", &name), &data, |b, d| {
            b.iter(|| next::write_string(black_box(d), &writer).unwrap());
        });
        group.bench_with_input(BenchmarkId::new("current", &name), &current, |b, d| {
            b.iter(|| writer.write_to_string(black_box(d)).unwrap());
        });
    }
    group.finish();
}

/// The peak resident set of this process, in bytes (Linux).
fn peak_rss() -> Option<u64> {
    let status = fs::read_to_string("/proc/self/status").ok()?;
    let line = status.lines().find(|l| l.starts_with("VmHWM:"))?;
    let kb: u64 = line.split_whitespace().nth(1)?.parse().ok()?;
    Some(kb * 1024)
}

/// One pipeline, one read, measured: see the module documentation.
fn measure(pipeline: &str) {
    let path = std::env::var("MODEL_FILE").unwrap_or_else(|_| {
        let people = std::env::var("MODEL_PEOPLE")
            .ok()
            .and_then(|p| p.parse().ok())
            .unwrap_or(330_000);
        let path = std::env::temp_dir().join(format!("ged_io-model-{people}.ged"));
        if !path.exists() {
            fs::write(&path, generated(people)).expect("write the generated file");
        }
        path.to_string_lossy().into_owned()
    });
    let bytes = fs::read(&path).expect("read the file");
    let size = bytes.len();
    let before = peak_rss().unwrap_or(0);
    let start = Instant::now();
    let kept: Box<dyn std::any::Any> = match pipeline {
        "tree" => Box::new(Tree::from_bytes(bytes)),
        "next" => Box::new(next::read_bytes(bytes)),
        "current" => Box::new(GedcomBuilder::new().build_from_bytes(&bytes).unwrap()),
        other => panic!("unknown pipeline {other}"),
    };
    let elapsed = start.elapsed().as_secs_f64();
    let peak = peak_rss().unwrap_or(0);
    let live = LIVE.load(Ordering::Relaxed);
    black_box(&kept);
    let mb = size as f64 / 1e6;
    println!(
        "{pipeline}: read {mb:.1} MB in {elapsed:.3} s ({:.0} MB/s); peak RSS {:.0} MB = {:.2}x the input (before reading: {:.0} MB); kept {:.0} MB = {:.2}x the input",
        mb / elapsed,
        peak as f64 / 1e6,
        peak as f64 / size as f64,
        before as f64 / 1e6,
        live as f64 / 1e6,
        live as f64 / size as f64,
    );
    if let Some(data) = kept.downcast_ref::<next::Dataset>() {
        println!(
            "{pipeline}: of which the store (input, pieces, identifiers) {:.0} MB",
            data.store.heap_size() as f64 / 1e6
        );
    }
    let writer = GedcomWriter::new();
    let start = Instant::now();
    let written = if let Some(tree) = kept.downcast_ref::<Tree>() {
        let mut out = Vec::new();
        writer.write_tree(&mut out, tree).unwrap();
        out.len()
    } else if let Some(data) = kept.downcast_ref::<next::Dataset>() {
        next::write_string(data, &writer).unwrap().len()
    } else if let Some(data) = kept.downcast_ref::<ged_io::types::GedcomData>() {
        writer.write_to_string(data).unwrap().len()
    } else {
        0
    };
    let elapsed = start.elapsed().as_secs_f64();
    println!(
        "{pipeline}: wrote {:.1} MB in {elapsed:.3} s ({:.0} MB/s); peak RSS {:.0} MB",
        written as f64 / 1e6,
        written as f64 / 1e6 / elapsed,
        peak_rss().unwrap_or(0) as f64 / 1e6,
    );
}

criterion_group!(benches, bench_read, bench_write);

fn main() {
    if let Ok(pipeline) = std::env::var("MODEL_RSS") {
        measure(&pipeline);
        return;
    }
    benches();
    Criterion::default().configure_from_args().final_summary();
}
