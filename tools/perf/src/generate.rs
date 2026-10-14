//! The generated dataset of the benchmarks, the performance tool and the
//! memory job of the CI: fictitious and deterministic.

use std::fmt::Write as _;

/// A fictitious 5.5.1 dataset of `n` individuals, `n / 2` families and
/// `n / 10` sources, with notes long enough to be continued, places, dates
/// and citations: about 470 bytes an individual.
#[must_use]
pub fn generated(n: usize) -> String {
    generated_by(n, "")
}

/// [`generated`] as the producer `producer` writes it, with the extension
/// of its own that its import profile interprets in every individual:
/// `ftm` (Family Tree Maker: a military service, `_MILT` with a date and a
/// place, read as an event), `paf` (Personal Ancestral File: a `_UID`,
/// read as 7.0's `UID`); any other name: [`generated`]. About 20 and 40
/// bytes more an individual.
#[must_use]
pub fn generated_by(n: usize, producer: &str) -> String {
    let source = match producer {
        "ftm" => "1 SOUR FTM\n",
        "paf" => "1 SOUR PAF\n",
        _ => "",
    };
    let mut s =
        format!("0 HEAD\n{source}1 GEDC\n2 VERS 5.5.1\n2 FORM LINEAGE-LINKED\n1 CHAR UTF-8\n");
    for i in 0..n {
        let _ = writeln!(s, "0 @I{i}@ INDI");
        match producer {
            "ftm" => {
                let _ = writeln!(s, "1 _MILT\n2 DATE {}\n2 PLAC Sampletown", 1820 + i % 80);
            }
            "paf" => {
                let _ = writeln!(s, "1 _UID {:032X}", 12_345 + i * 7_919);
            }
            _ => {}
        }
        let _ = write!(
            s,
            "1 NAME Given{i} /Family{}/\n1 SEX {}\n1 BIRT\n2 DATE {} JAN {}\n2 PLAC Sampletown, Region {}\n2 SOUR @S{}@\n3 PAGE p. {i}\n1 FAMS @F{}@\n1 NOTE {}\n2 CONT second line with an @@ sign\n1 CHAN\n2 DATE 1 JAN 2000\n",
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
