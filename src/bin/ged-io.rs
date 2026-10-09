//! `ged-io`: inspect, validate and rewrite GEDCOM files.

use std::fmt::Write as _;
use std::fs;
use std::io::{self, Write as _};
use std::process::ExitCode;

use ged_io::model::{Dataset, Event, Individual, RecordRef, Sex};
use ged_io::{GedcomBuilder, GedcomError, GedcomVersion, GedcomWriter, WriteError};

const HELP: &str = "\
ged-io - inspect, validate and rewrite GEDCOM files (5.5.1, 7.0, 7.1)

USAGE:
  ged-io <file.ged>
  ged-io --individual <XREF> <file.ged>
  ged-io --individual-lastname <LASTNAME> <file.ged>
  ged-io --individual-firstname <FIRSTNAME> <file.ged>
  ged-io --validate [--validation-level <LEVEL>] <file.ged>
  ged-io --write <VERSION> <file.ged>

OPTIONS:
  -h, --help                         Print this help
  --individual <XREF>                Display one individual (e.g. @I1@)
  --individual-lastname <LASTNAME>   List the individuals whose surname contains LASTNAME
                                     (case-insensitive)
  --individual-firstname <FIRSTNAME> List the individuals whose given names contain
                                     FIRSTNAME (case-insensitive)
  --validate                         Check the file against the GEDCOM specification of the
                                     version it declares (5.5.1, 7.0 or 7.1) and list every
                                     deviation with its line
  --validation-level <LEVEL>         strict: deviations are errors (exit code 2);
                                     lenient (default): they are warnings (exit code 0)
  --write <VERSION>                  Write the file to standard output as conformant GEDCOM
                                     5.5.1, 7.0 or 7.1, and list the repairs on standard error

NOTES:
  Any encoding is read. If both --individual-lastname and --individual-firstname are set,
  the individuals matching both are listed.

EXIT CODES:
  0 success, 1 I/O error, 2 validation errors, 3 usage error
";

#[derive(Debug, Default)]
struct Args {
    file: Option<String>,
    individual: Option<String>,
    lastname: Option<String>,
    firstname: Option<String>,
    validate: bool,
    strict: Option<bool>,
    write: Option<GedcomVersion>,
    help: bool,
}

#[derive(Debug)]
enum CliError {
    Io(io::Error),
    Read(GedcomError),
    Write(WriteError),
    Usage(String),
}

impl CliError {
    fn exit_code(&self) -> u8 {
        match self {
            CliError::Io(_) | CliError::Read(_) | CliError::Write(_) => 1,
            CliError::Usage(_) => 3,
        }
    }
}

impl std::fmt::Display for CliError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            CliError::Io(e) => write!(f, "I/O error: {e}"),
            CliError::Read(e) => write!(f, "{e}"),
            CliError::Write(e) => write!(f, "{e}"),
            CliError::Usage(msg) => write!(f, "usage error: {msg}"),
        }
    }
}

impl From<io::Error> for CliError {
    fn from(e: io::Error) -> Self {
        CliError::Io(e)
    }
}

fn usage(msg: impl Into<String>) -> CliError {
    CliError::Usage(msg.into())
}

fn parse_args(argv: &[String]) -> Result<Args, CliError> {
    let mut args = Args::default();
    let mut it = argv.iter().skip(1);
    while let Some(arg) = it.next() {
        let mut value = |name: &str, what: &str| {
            it.next()
                .cloned()
                .ok_or_else(|| usage(format!("{name} expects {what}")))
        };
        match arg.as_str() {
            "-h" | "--help" => args.help = true,
            "--individual" => args.individual = Some(value(arg, "an XREF")?),
            "--individual-lastname" => args.lastname = Some(value(arg, "a LASTNAME")?),
            "--individual-firstname" => args.firstname = Some(value(arg, "a FIRSTNAME")?),
            "--validate" => args.validate = true,
            "--validation-level" => {
                args.strict = Some(match value(arg, "strict or lenient")?.as_str() {
                    "strict" => true,
                    "lenient" => false,
                    other => {
                        return Err(usage(format!(
                            "unknown validation level: {other} (expected strict or lenient)"
                        )))
                    }
                });
            }
            "--write" => {
                args.write = Some(match value(arg, "a version")?.as_str() {
                    "5.5.1" => GedcomVersion::V5_5_1,
                    "7.0" => GedcomVersion::V7_0,
                    "7.1" => GedcomVersion::V7_1,
                    other => {
                        return Err(usage(format!(
                            "unknown version: {other} (expected 5.5.1, 7.0 or 7.1)"
                        )))
                    }
                });
            }
            other if other.starts_with('-') => {
                return Err(usage(format!("unknown option: {other}")));
            }
            file => {
                if args.file.is_some() {
                    return Err(usage(
                        "expected one file (quote a path that contains spaces)",
                    ));
                }
                args.file = Some(file.to_string());
            }
        }
    }
    Ok(args)
}

fn main() -> ExitCode {
    let argv: Vec<String> = std::env::args().collect();
    match run(&argv) {
        Ok(code) => code,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::from(e.exit_code())
        }
    }
}

fn run(argv: &[String]) -> Result<ExitCode, CliError> {
    let args = parse_args(argv)?;
    if args.help {
        print!("{HELP}");
        return Ok(ExitCode::SUCCESS);
    }
    let file = args
        .file
        .as_deref()
        .ok_or_else(|| usage("missing file (see --help)"))?;
    let filters = args.individual.is_some() || args.lastname.is_some() || args.firstname.is_some();
    if args.strict.is_some() && !args.validate {
        return Err(usage("--validation-level requires --validate"));
    }
    if [args.validate, filters, args.write.is_some()]
        .iter()
        .filter(|&&b| b)
        .count()
        > 1
    {
        return Err(usage(
            "--validate, --write and the --individual options cannot be combined",
        ));
    }
    let bytes = fs::read(file)?;
    if args.validate {
        return Ok(validate(&bytes, args.strict.unwrap_or(false)));
    }
    let data = GedcomBuilder::new()
        .build_from_bytes(bytes)
        .map_err(CliError::Read)?;
    let mut out = io::stdout().lock();
    if let Some(version) = args.write {
        let report = GedcomWriter::new()
            .gedcom_version(version)
            .write(&mut out, &data)
            .map_err(CliError::Write)?;
        for repair in &report.repairs {
            eprintln!("repair: {repair}");
        }
        return Ok(ExitCode::SUCCESS);
    }
    let text = if let Some(xref) = &args.individual {
        let individual = data
            .find_individual(xref.as_str())
            .ok_or_else(|| usage(format!("no individual {xref}")))?;
        describe(&data, individual)
    } else if filters {
        let wanted = |filter: &Option<String>, pieces: Vec<String>| {
            filter.as_ref().is_none_or(|f| {
                let f = f.to_lowercase();
                pieces.iter().any(|p| p.to_lowercase().contains(&f))
            })
        };
        let mut text = String::new();
        for individual in &data.individuals {
            let (given, surname) = name_parts(&data, individual);
            if wanted(&args.lastname, surname) && wanted(&args.firstname, given) {
                text.push_str(&describe(&data, individual));
            }
        }
        text
    } else {
        summary(&data)
    };
    out.write_all(text.as_bytes())?;
    Ok(ExitCode::SUCCESS)
}

/// Every deviation of the file from its specification, as errors in strict
/// mode, as warnings otherwise.
fn validate(bytes: &[u8], strict: bool) -> ExitCode {
    let deviations = ged_io::spec::validate_bytes(bytes);
    let (level, kind) = if strict {
        ("strict", "error")
    } else {
        ("lenient", "warning")
    };
    let (errors, warnings) = if strict {
        (deviations.len(), 0)
    } else {
        (0, deviations.len())
    };
    println!("Validation: {level} - errors: {errors}, warnings: {warnings}");
    for d in &deviations {
        println!("{kind}: {d}");
    }
    if errors > 0 {
        ExitCode::from(2)
    } else {
        ExitCode::SUCCESS
    }
}

/// The version, encoding and records of a dataset.
fn summary(data: &Dataset) -> String {
    let mut text = String::new();
    let declared = data
        .declared_version()
        .map(|d| format!(" (declared {d})"))
        .unwrap_or_default();
    let _ = writeln!(text, "GEDCOM {}{declared}", data.version());
    for (name, count) in [
        ("individuals", data.individuals.len()),
        ("families", data.families.len()),
        ("sources", data.sources.len()),
        ("repositories", data.repositories.len()),
        ("multimedia objects", data.multimedia.len()),
        ("shared notes", data.notes.len()),
        ("submitters", data.submitters.len()),
        ("submissions", data.submissions.len()),
        ("other records", data.extra.len()),
    ] {
        let _ = writeln!(text, "  {name}: {count}");
    }
    let dangling = data.dangling_references().len();
    if dangling > 0 {
        let _ = writeln!(text, "  pointers to no record: {dangling}");
    }
    text
}

/// The given names and surnames of an individual's names: their pieces,
/// or the name's words, its surname between slashes.
fn name_parts(data: &Dataset, individual: &Individual) -> (Vec<String>, Vec<String>) {
    let mut given = Vec::new();
    let mut surname = Vec::new();
    for name in &individual.names {
        given.extend(name.givens().map(|g| g.to_str(data).into_owned()));
        surname.extend(name.surnames().map(|s| s.to_str(data).into_owned()));
        let value = name.value.to_str(data);
        if let Some(s) = name.surname_in_value(data) {
            surname.push(s);
        }
        let before = value.split('/').next().unwrap_or_default().trim();
        if !before.is_empty() {
            given.push(before.to_string());
        }
    }
    (given, surname)
}

/// An event's kind, date and place.
fn event_line(data: &Dataset, label: &str, event: &Event) -> String {
    let mut line = format!("  {label}:");
    if let Some(date) = &event.date {
        let _ = write!(line, " {}", date.value.to_str(data));
    }
    if let Some(place) = &event.place {
        let _ = write!(line, ", {}", place.name.to_str(data));
    }
    line.push('\n');
    line
}

/// An individual: names, sex, birth and death, families.
fn describe(data: &Dataset, individual: &Individual) -> String {
    let mut text = String::new();
    let xref = individual
        .xref
        .map_or("(no identifier)", |x| data.store().xref(x));
    let name = individual
        .full_name(data)
        .unwrap_or_else(|| "(no name)".to_string());
    let _ = writeln!(text, "{xref} {name}");
    for other in individual.names.iter().skip(1) {
        let _ = writeln!(text, "  also: {}", other.full(data));
    }
    if let Some(sex) = &individual.sex {
        let sex = match sex {
            Sex::Male => "male",
            Sex::Female => "female",
            _ => "other or unknown",
        };
        let _ = writeln!(text, "  sex: {sex}");
    }
    if let Some(birth) = individual.birth() {
        text.push_str(&event_line(data, "born", birth));
    }
    if let Some(death) = individual.death() {
        text.push_str(&event_line(data, "died", death));
    }
    for family in data.families_as_child(individual.xref) {
        let parents: Vec<String> = data
            .parents(family)
            .filter_map(|p| p.full_name(data))
            .collect();
        if !parents.is_empty() {
            let _ = writeln!(text, "  child of: {}", parents.join(" and "));
        }
    }
    for family in data.families_as_spouse(individual.xref) {
        if let Some(spouse) = data.spouse(individual.xref, family) {
            let _ = writeln!(
                text,
                "  partner: {}",
                spouse.full_name(data).unwrap_or_default()
            );
        }
        for child in data.children(family) {
            let _ = writeln!(
                text,
                "  child: {}",
                child.full_name(data).unwrap_or_default()
            );
        }
    }
    let records = data
        .records()
        .filter(|r| matches!(r, RecordRef::Individual(i) if i.xref == individual.xref))
        .count();
    if records > 1 {
        let _ = writeln!(text, "  ({records} records have this identifier)");
    }
    text
}
