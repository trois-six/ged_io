//! Reading and writing time and peak memory of the published ged_io 0.17.0
//! and this checkout, on the same file, one crate and one operation per
//! process (so that each peak resident set is its own):
//!
//! ```sh
//! perf <old|new> <read|write> <file> [iterations]
//! perf generate <individuals> <file>
//! ```
//!
//! `read` reads the file's bytes into the model; `write` reads it once, then
//! writes the model to a string. Each prints the median and the minimum
//! time of `iterations` runs (default 10), the throughput and the peak
//! resident set of the process (Linux `VmHWM`), as one tab-separated line.
//! Run under `valgrind --tool=cachegrind` with one iteration to count
//! instructions.

use std::fmt::Write as _;
use std::hint::black_box;
use std::time::Instant;

/// The peak resident set of this process, in bytes (Linux).
fn peak_rss() -> u64 {
    std::fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            let line = s.lines().find(|l| l.starts_with("VmHWM:"))?;
            line.split_whitespace().nth(1)?.parse::<u64>().ok()
        })
        .map_or(0, |kb| kb * 1024)
}

/// A fictitious 5.5.1 dataset of `n` individuals, `n / 2` families and
/// `n / 10` sources (the generator of the crate's benchmarks).
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

fn old_read(bytes: &[u8]) -> ged_io_published::types::GedcomData {
    ged_io_published::GedcomBuilder::new()
        .build_from_bytes(bytes)
        .expect("0.17 reads the file")
}

fn new_read(bytes: &[u8]) -> ged_io::Dataset {
    ged_io::GedcomBuilder::new()
        .build_from_bytes(bytes)
        .expect("reading never fails")
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("generate") {
        let n = args.get(1).and_then(|n| n.parse().ok()).unwrap_or(330_000);
        let path = args.get(2).expect("perf generate <individuals> <file>");
        std::fs::write(path, generated(n)).expect("write the file");
        return;
    }
    let [krate, op, path, rest @ ..] = args.as_slice() else {
        eprintln!("perf <old|new> <read|write> <file> [iterations]");
        std::process::exit(2);
    };
    let iterations: usize = rest.first().and_then(|n| n.parse().ok()).unwrap_or(10);
    let bytes = std::fs::read(path).expect("read the file");
    let mut times = Vec::with_capacity(iterations);
    let mut out_len = 0;
    match (krate.as_str(), op.as_str()) {
        ("old", "read") => {
            for _ in 0..iterations {
                let t = Instant::now();
                black_box(old_read(black_box(&bytes)));
                times.push(t.elapsed().as_secs_f64());
            }
        }
        ("new", "read") => {
            for _ in 0..iterations {
                let t = Instant::now();
                black_box(new_read(black_box(&bytes)));
                times.push(t.elapsed().as_secs_f64());
            }
        }
        ("old", "write") => {
            let data = old_read(&bytes);
            for _ in 0..iterations {
                let t = Instant::now();
                let out = ged_io_published::GedcomWriter::new()
                    .write_to_string(black_box(&data))
                    .expect("0.17 writes");
                out_len = out.len();
                black_box(out);
                times.push(t.elapsed().as_secs_f64());
            }
        }
        ("new", "write") => {
            let data = new_read(&bytes);
            for _ in 0..iterations {
                let t = Instant::now();
                let out = ged_io::GedcomWriter::new()
                    .write_to_string(black_box(&data))
                    .expect("writes");
                out_len = out.len();
                black_box(out);
                times.push(t.elapsed().as_secs_f64());
            }
        }
        _ => {
            eprintln!("perf <old|new> <read|write> <file> [iterations]");
            std::process::exit(2);
        }
    }
    times.sort_by(f64::total_cmp);
    let median = times[times.len() / 2];
    let min = times[0];
    let size = if op == "write" { out_len } else { bytes.len() };
    let name = path.rsplit('/').next().unwrap_or(path);
    println!(
        "{krate}\t{op}\t{name}\t{:.3} ms\t{:.3} ms\t{:.1} MB/s\t{:.1} MB\t{:.2}x",
        median * 1e3,
        min * 1e3,
        size as f64 / 1e6 / median,
        peak_rss() as f64 / 1e6,
        peak_rss() as f64 / bytes.len() as f64,
    );
}
