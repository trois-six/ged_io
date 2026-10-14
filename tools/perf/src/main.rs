//! Time, throughput and peak memory of ged_io's pipelines, one pipeline per
//! process (so that each peak resident set is its own).
//!
//! ```sh
//! perf generate <individuals> <file> [ftm|paf] # the generated dataset
//! perf run <pipeline> <file> [iterations]      # one pipeline, measured
//! perf check <file> [ftm=<file>] [paf=<file>] [threshold=value…]
//! ```
//!
//! Pipelines: `read` (the bytes into a `Dataset`, with the default
//! lossless interpretation of the producer's extensions), `read-none` (the
//! same, without interpretation), `write` (read once, then
//! `write_to_string`, timed), `write-v7` (the same, in GEDCOM 7.0),
//! `export` (read once, then the `geneanet` export profile's
//! `write_to_string`, timed), `stream` (`GedcomStreamParser` over the
//! file), `stream-write` (`GedcomStreamParser` into a
//! `GedcomStreamWriter`), `tree` (the lossless `Tree`). `run` prints one
//! tab-separated line: the pipeline, the file, the median and minimum time
//! of `iterations` runs (default 5), the throughput, the peak resident set
//! (Linux `VmHWM`) and its ratio to the file's size.
//!
//! `generate` writes the generated dataset (see `generate.rs`), or, with
//! `ftm` or `paf`, the same as that producer writes it, with the extension
//! of its own its import profile interprets in every individual.
//!
//! `check` runs each pipeline once in a child process and fails (exit 1)
//! when a peak or a throughput passes its threshold. The thresholds, which
//! the CI's memory job checks, default to the redesign's targets: a
//! dataset read peaks at most at 3 times the input (`read-ratio=3`), its
//! write at 4.5 (`write-ratio=4.5`, the output being held whole); a stream
//! read or write stays under 64 MiB beyond the largest record
//! (`stream-mib=64`); the in-memory pipelines run at 50 MB/s at least
//! (`min-mbps=50`), the streams at 20 (`min-stream-mbps=20`): generous
//! floors, CI machines vary.
//!
//! The pipelines of the producer profiles are checked against the plain
//! ones on the same machine, by the median of three runs: the `geneanet`
//! export of the file against its write (`export-time=2`); and, given the
//! files the `ftm` and `paf` generators write (`ftm=…`, `paf=…`), each
//! read interpreted against its read without interpretation
//! (`profile-read-time=2`), each 7.0 write against the plain file's 7.0
//! write (`profile-write-time=2`). Each peaks within the budget of its
//! plain pipeline: an export or a 7.0 write at `write-ratio`, a read at
//! `read-ratio`. The profiles run at about 1.5 times their plain pipeline
//! in instructions; the time thresholds leave room for the machine, so
//! that a regression fails.

mod generate;

use std::fs;
use std::hint::black_box;
use std::io::{BufReader, Write as _};
use std::process::{Command, ExitCode};
use std::time::Instant;

use ged_io::model::RecordRef;
use ged_io::profile::{Exporter, Interpretation};
use ged_io::tree::Tree;
use ged_io::{
    GedcomBuilder, GedcomStreamParser, GedcomStreamWriter, GedcomVersion, GedcomWriter,
    WriterConfig,
};

/// The peak resident set of this process, in bytes (Linux `VmHWM`); 0
/// elsewhere.
fn peak_rss() -> u64 {
    fs::read_to_string("/proc/self/status")
        .ok()
        .and_then(|s| {
            let line = s.lines().find(|l| l.starts_with("VmHWM:"))?;
            line.split_whitespace().nth(1)?.parse::<u64>().ok()
        })
        .map_or(0, |kb| kb * 1024)
}

/// Reads `path` into a dataset, interpreting its producer's extensions
/// at `level`.
fn read_at(path: &str, level: Interpretation) -> ged_io::Dataset {
    GedcomBuilder::new()
        .interpretation(level)
        .build_from_bytes(fs::read(path).expect("read the file"))
        .expect("reading never fails")
}

/// Reads `path` into a dataset, as the builder does by default.
fn read(path: &str) -> ged_io::Dataset {
    read_at(path, Interpretation::default())
}

/// Streams `path` record by record, into a stream writer when `write`.
fn stream(path: &str, write: bool) -> usize {
    let file = BufReader::new(fs::File::open(path).expect("open the file"));
    let mut records = GedcomStreamParser::new(file).expect("read the file");
    if !write {
        return records.fold(0, |n, r| {
            r.expect("reads");
            n + 1
        });
    }
    let Some(head) = records.next().map(|r| r.expect("reads")) else {
        return 0;
    };
    let default = ged_io::model::Header::default();
    let header = match head.record() {
        RecordRef::Header(h) => h.header(),
        _ => &default,
    };
    let mut writer =
        GedcomStreamWriter::new(std::io::sink(), WriterConfig::default(), header, &head)
            .expect("writes");
    let mut n = 1;
    for record in records {
        let record = record.expect("reads");
        writer.write_record(record.record()).expect("writes");
        n += 1;
    }
    writer.finish().expect("writes");
    n
}

/// One pipeline, `iterations` times: the line `run` prints.
fn run(pipeline: &str, path: &str, iterations: usize) -> Result<String, String> {
    let size = fs::metadata(path).map_err(|e| e.to_string())?.len();
    let mut times = Vec::with_capacity(iterations);
    let mut out_len = size;
    match pipeline {
        "read" | "read-none" => {
            let level = if pipeline == "read" {
                Interpretation::default()
            } else {
                Interpretation::None
            };
            for _ in 0..iterations {
                let t = Instant::now();
                black_box(read_at(path, level));
                times.push(t.elapsed().as_secs_f64());
            }
        }
        "export" => {
            let data = read(path);
            let exporter = Exporter::named("geneanet").expect("a shipped profile");
            for _ in 0..iterations {
                let t = Instant::now();
                let (out, report) = exporter.write_to_string(black_box(&data)).expect("writes");
                out_len = out.len() as u64;
                black_box((out, report));
                times.push(t.elapsed().as_secs_f64());
            }
        }
        "write" | "write-v7" => {
            let data = read(path);
            let writer = if pipeline == "write" {
                GedcomWriter::new()
            } else {
                GedcomWriter::new().gedcom_version(GedcomVersion::V7_0)
            };
            for _ in 0..iterations {
                let t = Instant::now();
                let out = writer.write_to_string(black_box(&data)).expect("writes");
                out_len = out.len() as u64;
                black_box(out);
                times.push(t.elapsed().as_secs_f64());
            }
        }
        "stream" | "stream-write" => {
            for _ in 0..iterations {
                let t = Instant::now();
                black_box(stream(path, pipeline == "stream-write"));
                times.push(t.elapsed().as_secs_f64());
            }
        }
        "tree" => {
            for _ in 0..iterations {
                let t = Instant::now();
                black_box(Tree::from_bytes(fs::read(path).expect("read the file")));
                times.push(t.elapsed().as_secs_f64());
            }
        }
        other => return Err(format!("unknown pipeline {other}")),
    }
    times.sort_by(f64::total_cmp);
    let median = times.get(times.len() / 2).copied().unwrap_or(0.0);
    let min = times.first().copied().unwrap_or(0.0);
    let peak = peak_rss();
    Ok(format!(
        "{pipeline}\t{}\t{:.3} ms\t{:.3} ms\t{:.1} MB/s\t{:.1} MB\t{:.2}x",
        path.rsplit('/').next().unwrap_or(path),
        median * 1e3,
        min * 1e3,
        out_len as f64 / 1e6 / median,
        peak as f64 / 1e6,
        peak as f64 / size as f64,
    ))
}

/// A field of a line `run` printed, as a number.
fn field(line: &str, i: usize) -> f64 {
    line.split('\t')
        .nth(i)
        .and_then(|f| f.split([' ', 'x']).next())
        .and_then(|f| f.parse().ok())
        .unwrap_or(f64::NAN)
}

/// Every pipeline once, each in a child process, against the thresholds.
fn check(path: &str, args: &[String]) -> Result<bool, String> {
    let mut limits = [
        ("read-ratio", 3.0),
        ("write-ratio", 4.5),
        ("stream-mib", 64.0),
        ("min-mbps", 50.0),
        ("min-stream-mbps", 20.0),
        ("export-time", 2.0),
        ("profile-read-time", 2.0),
        ("profile-write-time", 2.0),
    ];
    // The files of the producers, by name.
    let mut producers: Vec<(&str, &str)> = Vec::new();
    for arg in args {
        let (name, value) = arg
            .split_once('=')
            .ok_or_else(|| format!("{arg}: threshold=value"))?;
        if matches!(name, "ftm" | "paf") {
            producers.push((name, value));
            continue;
        }
        let slot = limits
            .iter_mut()
            .find(|(n, _)| *n == name)
            .ok_or_else(|| format!("unknown threshold {name}"))?;
        slot.1 = value.parse().map_err(|_| format!("{arg}: not a number"))?;
    }
    let limit = |name: &str| {
        limits
            .iter()
            .find(|(n, _)| *n == name)
            .map_or(f64::NAN, |l| l.1)
    };
    let size = fs::metadata(path).map_err(|e| e.to_string())?.len() as f64;
    let mut ok = true;
    for pipeline in ["read", "write", "stream", "stream-write", "tree"] {
        let line = run_child(pipeline, path, 1)?;
        let (mbps, peak_mb, ratio) = (field(&line, 4), field(&line, 5), field(&line, 6));
        let mut failed = Vec::new();
        match pipeline {
            "read" | "tree" if ratio > limit("read-ratio") => failed.push("peak"),
            "write" if ratio > limit("write-ratio") => failed.push("peak"),
            "stream" | "stream-write" if peak_mb * 1e6 > limit("stream-mib") * 1048576.0 => {
                failed.push("peak");
            }
            _ => {}
        }
        let floor = if pipeline.starts_with("stream") {
            limit("min-stream-mbps")
        } else {
            limit("min-mbps")
        };
        if mbps < floor {
            failed.push("throughput");
        }
        ok &= failed.is_empty();
        println!(
            "{line}\t{}",
            if failed.is_empty() {
                "ok".to_string()
            } else {
                format!("FAILED: {}", failed.join(", "))
            }
        );
    }
    println!("input: {:.1} MB", size / 1e6);
    // The profiles, against the plain pipelines.
    let mut relative = |what: &str, line: &str, base: &str, time: f64, peak: f64| {
        let times = field(line, 2) / field(base, 2);
        let mut failed = Vec::new();
        if times > time {
            failed.push("time");
        }
        if field(line, 6) > peak {
            failed.push("peak");
        }
        ok &= failed.is_empty();
        println!(
            "{line}\t{times:.2}x {what}\t{}",
            if failed.is_empty() {
                "ok".to_string()
            } else {
                format!("FAILED: {}", failed.join(", "))
            }
        );
    };
    let write = run_child("write", path, 3)?;
    let export = run_child("export", path, 3)?;
    relative(
        "the write",
        &export,
        &write,
        limit("export-time"),
        limit("write-ratio"),
    );
    if !producers.is_empty() {
        let plain_v7 = run_child("write-v7", path, 3)?;
        for (name, file) in producers {
            let none = run_child("read-none", file, 3)?;
            let read = run_child("read", file, 3)?;
            let what = format!("{name}: its read without interpretation");
            let (time, peak) = (limit("profile-read-time"), limit("read-ratio"));
            relative(&what, &read, &none, time, peak);
            let v7 = run_child("write-v7", file, 3)?;
            let what = format!("{name}: the plain file's 7.0 write");
            let (time, peak) = (limit("profile-write-time"), limit("write-ratio"));
            relative(&what, &v7, &plain_v7, time, peak);
        }
    }
    Ok(ok)
}

/// The line `run` prints of `pipeline` on `path`, `iterations` times, in a
/// child process (so that its peak is its own).
fn run_child(pipeline: &str, path: &str, iterations: usize) -> Result<String, String> {
    let exe = std::env::current_exe().map_err(|e| e.to_string())?;
    let output = Command::new(&exe)
        .args(["run", pipeline, path, &iterations.to_string()])
        .output()
        .map_err(|e| e.to_string())?;
    if !output.status.success() {
        return Err(format!(
            "{pipeline} failed: {}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
}

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let usage = "perf generate <individuals> <file> [ftm|paf] | perf run <pipeline> <file> [iterations] | perf check <file> [ftm=<file>] [paf=<file>] [threshold=value…]";
    let result = match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["generate", n, path, producer @ ..] => n
            .parse::<usize>()
            .map_err(|e| e.to_string())
            .and_then(|n| {
                let text = match producer.first() {
                    Some(producer) => generate::generated_by(n, producer),
                    None => generate::generated(n),
                };
                fs::write(path, text).map_err(|e| e.to_string())
            })
            .map(|()| true),
        ["run", pipeline, path, rest @ ..] => {
            let iterations = rest.first().and_then(|n| n.parse().ok()).unwrap_or(5);
            run(pipeline, path, iterations).map(|line| {
                println!("{line}");
                true
            })
        }
        ["check", path, ..] => check(path, args.get(2..).unwrap_or(&[])),
        _ => Err(usage.to_string()),
    };
    let _ = std::io::stdout().flush();
    match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("{e}");
            ExitCode::from(2)
        }
    }
}
