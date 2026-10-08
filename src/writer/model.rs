//! Writing the typed model ([`GedcomData`]) through the emitter.
//!
//! Each structure of the model is written by one function below, which
//! hands the emitter its level, tag and payload, typed: pointer fields as
//! pointers (mapped by the [`XrefMap`]), everything else as text. Which
//! structures exist in which version is still decided here, structure by
//! structure; the generated specification tables take that over later.

use std::borrow::Cow;

use super::emit::LineSink;
use super::head;
use super::xref::{RecordXref, XrefMap};
use super::{put_structure, Capture, Emitter, Out, WriteError};
use crate::tree::PayloadRef;
use crate::types::{
    address::Address,
    age::Age,
    custom::UserDefinedTag,
    date::Date,
    event::{detail::Detail as EventDetail, spouse::Spouse, Event},
    family::Family,
    gedcom7::{Crop, NonEvent, SortDate},
    header::{schema::Schema, source::HeadSour, Header},
    individual::{
        association::Association,
        attribute::detail::AttributeDetail,
        family_link::FamilyLink,
        gender::{Gender, GenderType},
        name::Name,
        Individual,
    },
    lds::LdsOrdinance,
    multimedia::Multimedia,
    note::Note,
    repository::Repository,
    shared_note::SharedNote,
    source::{
        citation::{Citation, CitationSource},
        data::Data as SourceData,
        quay::CertaintyAssessment,
        Source,
    },
    submission::Submission,
    submitter::Submitter,
    GedcomData,
};
use crate::util::is_xref_pointer;
use crate::version::VersionRules;

/// The records of `data` in write order, as the [`XrefMap`] sees them. The
/// order must be the one [`write_data`] writes them in.
fn records(data: &GedcomData) -> impl Iterator<Item = RecordXref<'_>> + Clone {
    fn of<'a>(
        prefix: &'static str,
        xrefs: impl Iterator<Item = Option<&'a String>> + Clone,
    ) -> impl Iterator<Item = RecordXref<'a>> + Clone {
        xrefs.map(move |x| (Some(prefix), x.map(String::as_str)))
    }
    of("U", data.submitters.iter().map(|r| r.xref.as_ref()))
        .chain(of("SUBN", data.submissions.iter().map(|r| r.xref.as_ref())))
        .chain(of("I", data.individuals.iter().map(|r| r.xref.as_ref())))
        .chain(of("F", data.families.iter().map(|r| r.xref.as_ref())))
        .chain(of("S", data.sources.iter().map(|r| r.xref.as_ref())))
        .chain(of("R", data.repositories.iter().map(|r| r.xref.as_ref())))
        .chain(of("M", data.multimedia.iter().map(|r| r.xref.as_ref())))
        .chain(of("N", data.shared_notes.iter().map(|r| r.xref.as_ref())))
        .chain(data.custom_data.iter().map(|r| (None, r.xref.as_deref())))
}

/// Writes `data`: the completed header, every record, the trailer.
pub(crate) fn write_data(
    rules: &'static VersionRules,
    sink: &mut LineSink<'_>,
    data: &GedcomData,
) -> Result<(), WriteError> {
    let mut xrefs = XrefMap::new(rules, records(data));
    for repair in std::mem::take(&mut xrefs.repairs) {
        sink.repair(repair)?;
    }

    // The header is captured, completed for the target, then written.
    let mut capture = Capture::default();
    {
        let mut cx = Ctx::new(rules, &mut capture, &mut xrefs);
        cx.write_header(data.header.as_ref())?;
    }
    let mut head_record = capture.roots.into_iter().next().unwrap_or_default();
    let mut stub = None;
    let submitter = if head::needs_submitter(&head_record, rules) {
        // The first submitter record comes first in write order.
        let first = data
            .submitters
            .first()
            .and_then(|s| xrefs.record(0, s.xref.as_deref()))
            .map(str::to_string);
        Some(first.unwrap_or_else(|| {
            let xref = xrefs.fresh("U");
            stub = Some(head::stub_submitter(xref.clone()));
            xref
        }))
    } else {
        None
    };
    head::complete(&mut head_record, rules, sink.charset(), submitter);
    let mut out = Emitter { rules, sink };
    put_structure(&mut out, &head_record)?;
    if let Some(stub) = stub {
        put_structure(&mut out, &stub)?;
    }

    let mut cx = Ctx::new(rules, &mut out, &mut xrefs);
    for submitter in &data.submitters {
        cx.write_submitter(submitter)?;
    }
    for submission in &data.submissions {
        cx.write_submission(submission)?;
    }
    for individual in &data.individuals {
        cx.write_individual(individual)?;
    }
    for family in &data.families {
        cx.write_family(family)?;
    }
    for source in &data.sources {
        cx.write_source(source)?;
    }
    for repo in &data.repositories {
        cx.write_repository(repo)?;
    }
    for media in &data.multimedia {
        cx.write_multimedia(media)?;
    }
    for shared_note in &data.shared_notes {
        cx.write_shared_note(shared_note)?;
    }
    // Extension records (`0 _XXX`) and unknown records.
    cx.write_custom_data(0, &data.custom_data)?;
    cx.out.put(0, None, "TRLR", PayloadRef::None)
}

/// The state of one typed write: the target's rules, where structures go
/// and the record identifiers.
struct Ctx<'c, 'a> {
    rules: &'static VersionRules,
    out: &'c mut dyn Out,
    xrefs: &'c mut XrefMap<'a>,
    /// The position of the next record in write order.
    record: usize,
}

impl<'c, 'a> Ctx<'c, 'a> {
    fn new(rules: &'static VersionRules, out: &'c mut dyn Out, xrefs: &'c mut XrefMap<'a>) -> Self {
        Self {
            rules,
            out,
            xrefs,
            record: 0,
        }
    }

    /// Whether the target is GEDCOM 7.x.
    fn v7(&self) -> bool {
        self.rules.version.is_v7()
    }

    /// The position of the next record, counted.
    fn next_record(&mut self) -> usize {
        let index = self.record;
        self.record += 1;
        index
    }

    /// Writes a record line, with the identifier the map gives it.
    fn write_record(&mut self, tag: &str, xref: Option<&str>) -> Result<(), WriteError> {
        let index = self.next_record();
        let Ctx { out, xrefs, .. } = self;
        out.put(0, xrefs.record(index, xref), tag, PayloadRef::None)
    }

    /// Writes a record line with a text payload (a shared note record).
    fn write_record_text(
        &mut self,
        tag: &str,
        xref: Option<&str>,
        text: &str,
    ) -> Result<(), WriteError> {
        let index = self.next_record();
        let Ctx { out, xrefs, .. } = self;
        out.put(0, xrefs.record(index, xref), tag, PayloadRef::Text(text))
    }

    /// Writes a structure whose payload, if any, is text.
    fn write_line(
        &mut self,
        level: usize,
        tag: &str,
        value: Option<&str>,
    ) -> Result<(), WriteError> {
        let payload = value.map_or(PayloadRef::None, PayloadRef::Text);
        self.out.put(level, None, tag, payload)
    }

    /// Writes a structure with a text payload, continued as needed.
    fn write_value_or_wrap(
        &mut self,
        level: usize,
        tag: &str,
        value: Option<&str>,
    ) -> Result<(), WriteError> {
        self.write_line(level, tag, value)
    }

    /// Writes a structure with a text payload, continued as needed.
    fn write_long_text(&mut self, level: usize, tag: &str, text: &str) -> Result<(), WriteError> {
        self.write_line(level, tag, Some(text))
    }

    /// Writes a structure whose payload is a pointer field of the model. A
    /// value without the shape of a pointer is not one, and is written as
    /// text.
    fn write_pointer(&mut self, level: usize, tag: &str, value: &str) -> Result<(), WriteError> {
        if is_pointer_shaped(value) {
            let pointer = self.xrefs.pointer(value.trim());
            self.out
                .put(level, None, tag, PayloadRef::Pointer(&pointer))
        } else {
            self.write_line(level, tag, Some(value))
        }
    }

    /// Writes a structure whose payload is untyped (an extension's value): a
    /// pointer when it has the shape of one, text otherwise.
    fn put_untyped(
        &mut self,
        level: usize,
        xref: Option<&str>,
        tag: &str,
        value: Option<&str>,
    ) -> Result<(), WriteError> {
        match value {
            Some(v) if is_xref_pointer(v) => {
                let pointer = self.xrefs.pointer(v);
                self.out
                    .put(level, xref, tag, PayloadRef::Pointer(&pointer))
            }
            value => self.out.put(
                level,
                xref,
                tag,
                value.map_or(PayloadRef::None, PayloadRef::Text),
            ),
        }
    }

    /// Writes the data's header. `GEDC` and `CHAR` are left to
    /// [`head::complete`], which writes them for the target.
    fn write_header(&mut self, header: Option<&Header>) -> Result<(), WriteError> {
        self.write_line(0, "HEAD", None)?;
        let Some(header) = header else {
            return Ok(());
        };
        if let Some(ref source) = header.source {
            self.write_header_source(source)?;
        }
        if let Some(ref dest) = header.destination {
            self.write_value_or_wrap(1, "DEST", Some(dest))?;
        }
        if let Some(ref date) = header.date {
            self.write_date(1, date)?;
        }
        if let Some(ref subm) = header.submitter_tag {
            self.write_pointer(1, "SUBM", subm)?;
        }
        if let Some(ref file) = header.filename {
            self.write_value_or_wrap(1, "FILE", Some(file))?;
        }
        if let Some(ref copyright) = header.copyright {
            self.write_value_or_wrap(1, "COPR", Some(copyright))?;
        }
        if let Some(ref lang) = header.language {
            self.write_value_or_wrap(1, "LANG", Some(lang))?;
        }
        if let Some(ref note) = header.note {
            self.write_note(1, note)?;
        }
        if let Some(ref schema) = header.schema {
            self.write_schema(schema)?;
        }
        self.write_custom_data(1, &header.custom_data)
    }

    /// Writes the header source block.
    fn write_header_source(&mut self, source: &HeadSour) -> Result<(), WriteError> {
        self.write_line(1, "SOUR", source.value.as_deref())?;
        if let Some(ref version) = source.version {
            self.write_line(2, "VERS", Some(version))?;
        }
        if let Some(ref name) = source.name {
            self.write_value_or_wrap(2, "NAME", Some(name))?;
        }
        if let Some(ref corp) = source.corporation {
            self.write_value_or_wrap(2, "CORP", corp.value.as_deref())?;
            if let Some(ref addr) = corp.address {
                self.write_address(3, addr)?;
            }
        }
        if let Some(ref data) = source.data {
            self.write_value_or_wrap(2, "DATA", data.value.as_deref())?;
            if let Some(ref date) = data.date {
                self.write_date(3, date)?;
            }
            if let Some(ref copyright) = data.copyright {
                self.write_value_or_wrap(3, "COPR", Some(copyright))?;
            }
        }
        Ok(())
    }

    /// Writes an individual record.
    fn write_individual(&mut self, individual: &Individual) -> Result<(), WriteError> {
        self.write_record("INDI", individual.xref.as_deref())?;

        if let Some(ref restriction) = individual.restriction {
            self.write_value_or_wrap(1, "RESN", Some(restriction))?;
        }

        if !individual.names.is_empty() {
            for name in &individual.names {
                self.write_name(name)?;
            }
        }

        if let Some(ref sex) = individual.sex {
            self.write_gender(sex)?;
        }

        for event in &individual.events {
            self.write_event(1, event)?;
        }

        for attr in &individual.attributes {
            self.write_attribute(attr)?;
        }

        // GEDCOM 7.0: Non-events
        for non_event in &individual.non_events {
            self.write_non_event(1, non_event)?;
        }

        // LDS Ordinances (BAPL, CONL, INIL, ENDL, SLGC)
        for ordinance in &individual.lds_ordinances {
            self.write_lds_ordinance(1, ordinance)?;
        }

        for family_link in &individual.families {
            let tag = family_link.family_link_type.to_tag();
            self.write_pointer(1, tag, &family_link.xref)?;
            self.write_family_link_detail(2, family_link)?;
        }

        for submitter in &individual.submitters {
            self.write_pointer(1, "SUBM", submitter)?;
        }

        for citation in &individual.source {
            self.write_citation(1, citation)?;
        }

        // Associations (witnesses, godparents, ...), e.g. `1 ASSO @I2@` /
        // `2 RELA Witness` — must be a direct child of the INDI record per
        // the GEDCOM 5.5.1 grammar; nesting it inside an event (as the
        // `write_event` ASSO branch above does) is a non-standard extension
        // most readers, including Gramps, reject.
        for association in &individual.associations {
            self.write_association(1, association)?;
        }

        for alias in &individual.aliases {
            self.write_pointer(1, "ALIA", alias)?;
        }
        if let Some(ref ancestor_interest) = individual.ancestor_interest {
            self.write_pointer(1, "ANCI", ancestor_interest)?;
        }
        if let Some(ref descendant_interest) = individual.descendant_interest {
            self.write_pointer(1, "DESI", descendant_interest)?;
        }
        if let Some(ref afn) = individual.ancestral_file_number {
            self.write_value_or_wrap(1, "AFN", Some(afn))?;
        }
        self.write_record_identifiers(
            individual.user_reference_number.as_deref(),
            individual.user_reference_type.as_deref(),
            individual.automated_record_id.as_deref(),
            individual.uid.as_deref(),
            &individual.external_ids,
        )?;

        for media in &individual.multimedia {
            self.write_multimedia_link(1, media)?;
        }

        for note in &individual.notes {
            self.write_note(1, note)?;
        }

        if let Some(ref change_date) = individual.change_date {
            self.write_line(1, "CHAN", None)?;
            if let Some(ref date) = change_date.date {
                self.write_date(2, date)?;
            }
        }

        self.write_custom_data(1, &individual.custom_data)?;

        Ok(())
    }

    /// Writes a name structure.
    fn write_name(&mut self, name: &Name) -> Result<(), WriteError> {
        self.write_value_or_wrap(1, "NAME", name.value.as_deref())?;

        if let Some(ref name_type) = name.name_type {
            self.write_value_or_wrap(2, "TYPE", Some(name_type.as_str()))?;
        }

        if let Some(ref given) = name.given {
            self.write_value_or_wrap(2, "GIVN", Some(given))?;
        }

        if let Some(ref nickname) = name.nickname {
            self.write_value_or_wrap(2, "NICK", Some(nickname))?;
        }

        if let Some(ref surname) = name.surname {
            self.write_value_or_wrap(2, "SURN", Some(surname))?;
        }

        if let Some(ref prefix) = name.prefix {
            self.write_value_or_wrap(2, "NPFX", Some(prefix))?;
        }

        if let Some(ref suffix) = name.suffix {
            self.write_value_or_wrap(2, "NSFX", Some(suffix))?;
        }

        if let Some(ref surname_prefix) = name.surname_prefix {
            self.write_value_or_wrap(2, "SPFX", Some(surname_prefix))?;
        }

        // Source citations for name
        for citation in &name.source {
            self.write_citation(2, citation)?;
        }

        // Note
        for note in &name.notes {
            self.write_note(2, note)?;
        }

        self.write_custom_data(2, &name.custom_data)?;

        Ok(())
    }

    /// Writes a gender record.
    fn write_gender(&mut self, gender: &Gender) -> Result<(), WriteError> {
        let sex_char = match gender.value {
            GenderType::Male => "M",
            GenderType::Female => "F",
            GenderType::Nonbinary => "X",
            GenderType::Unknown => "U",
        };
        self.write_line(1, "SEX", Some(sex_char))?;

        if let Some(ref fact) = gender.fact {
            self.write_long_text(2, "FACT", fact)?;
        }

        for citation in &gender.sources {
            self.write_citation(2, citation)?;
        }

        self.write_custom_data(2, &gender.custom_data)?;

        Ok(())
    }

    /// Writes an event detail.
    fn write_event(&mut self, level: usize, event: &EventDetail) -> Result<(), WriteError> {
        let tag = event_to_tag(&event.event);
        self.write_line(level, tag, event.value.as_deref())?;

        if let Some(ref date) = event.date {
            self.write_date(level + 1, date)?;
        }

        // GEDCOM 7.0: Sort date
        if let Some(ref sort_date) = event.sort_date {
            self.write_sort_date(level + 1, sort_date)?;
        }

        if let Some(ref place) = event.place {
            self.write_value_or_wrap(level + 1, "PLAC", place.value.as_deref())?;
            if let Some(ref form) = place.form {
                self.write_value_or_wrap(level + 2, "FORM", Some(form))?;
            }
            if let Some(ref map) = place.map {
                self.write_line(level + 2, "MAP", None)?;
                if let Some(ref lat) = map.latitude {
                    self.write_value_or_wrap(level + 3, "LATI", Some(lat))?;
                }
                if let Some(ref lon) = map.longitude {
                    self.write_value_or_wrap(level + 3, "LONG", Some(lon))?;
                }
            }
            self.write_place_references(level + 2, place)?;
            for phonetic in &place.phonetic {
                self.write_value_or_wrap(level + 2, "FONE", Some(&phonetic.value))?;
                if let Some(ref vtype) = phonetic.variation_type {
                    self.write_value_or_wrap(level + 3, "TYPE", Some(vtype))?;
                }
            }
            for romanized in &place.romanized {
                self.write_value_or_wrap(level + 2, "ROMN", Some(&romanized.value))?;
                if let Some(ref vtype) = romanized.variation_type {
                    self.write_value_or_wrap(level + 3, "TYPE", Some(vtype))?;
                }
            }
            self.write_custom_data(level + 2, &place.custom_data)?;
        }

        self.write_event_address(level + 1, event)?;

        if let Some(ref event_type) = event.event_type {
            self.write_value_or_wrap(level + 1, "TYPE", Some(event_type))?;
        }

        for citation in &event.citations {
            self.write_citation(level + 1, citation)?;
        }

        for media in &event.multimedia {
            self.write_multimedia_link(level + 1, media)?;
        }

        for note in &event.notes {
            self.write_note(level + 1, note)?;
        }

        // New fields: CAUS, RESN, AGE, AGNC, RELI
        if let Some(ref cause) = event.cause {
            self.write_long_text(level + 1, "CAUS", cause)?;
        }

        if let Some(ref restriction) = event.restriction {
            self.write_value_or_wrap(level + 1, "RESN", Some(restriction))?;
        }

        if let Some(ref age) = event.age {
            self.write_age(level + 1, age)?;
        }

        if let Some(ref agency) = event.agency {
            self.write_value_or_wrap(level + 1, "AGNC", Some(agency))?;
        }

        if let Some(ref religion) = event.religion {
            self.write_value_or_wrap(level + 1, "RELI", Some(religion))?;
        }

        for detail in &event.family_event_details {
            let tag = match detail.member {
                Some(Spouse::Spouse1) => "HUSB",
                Some(Spouse::Spouse2) => "WIFE",
                None => continue,
            };
            self.write_line(level + 1, tag, None)?;
            if let Some(ref age) = detail.age {
                self.write_age(level + 2, age)?;
            }
        }

        // Adoptive/foster family link, e.g. `1 ADOP` / `2 FAMC @F1@` / `3 ADOP HUSB`.
        if let Some(ref family_link) = event.family_link {
            let tag = family_link.family_link_type.to_tag();
            self.write_pointer(level + 1, tag, &family_link.xref)?;
            self.write_family_link_detail(level + 2, family_link)?;
        }

        // Associations (witnesses, godparents, ...), e.g. `1 ASSO @I2@` / `2 RELA Godmother`.
        for association in &event.associations {
            self.write_association(level + 1, association)?;
        }

        self.write_custom_data(level + 1, &event.custom_data)?;

        Ok(())
    }

    /// Writes an `AGE` structure in the grammar of the target version.
    ///
    /// The age is converted by [`Age::to_version`]: GEDCOM 7.0 has no
    /// `CHILD`, `INFANT` or `STILLBORN` keywords, GEDCOM 5.5.1 no weeks and
    /// no `PHRASE`. An age known only as text is written as the 5.5.1
    /// payload, which is where 5.5.1 files carry it. An age with neither
    /// payload nor phrase is not written, as an empty `AGE` line is valid in
    /// neither version.
    fn write_age(&mut self, level: usize, age: &Age) -> Result<(), WriteError> {
        let version = self.rules.version;
        let gedcom_7 = version.is_v7();
        let age = age.to_version(version);
        let value = age
            .value
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty());
        let phrase = age.phrase.as_deref().filter(|p| !p.is_empty());
        let (payload, phrase) = match (value, phrase) {
            (None, None) => return Ok(()),
            (Some(value), phrase) => (value, phrase.filter(|_| gedcom_7)),
            (None, Some(phrase)) if gedcom_7 => ("", Some(phrase)),
            (None, Some(phrase)) => (phrase, None),
        };

        self.write_value_or_wrap(level, "AGE", Some(payload))?;
        if let Some(phrase) = phrase {
            self.write_value_or_wrap(level + 1, "PHRASE", Some(phrase))?;
        }
        Ok(())
    }

    /// Writes a family link's `PEDI`/`ADOP`/`NOTE` substructures (shared by
    /// the individual's own `FAMC`/`FAMS` back-links and an event's nested
    /// adoptive-family `FAMC`, e.g. under `ADOP`).
    fn write_family_link_detail(
        &mut self,
        level: usize,
        family_link: &FamilyLink,
    ) -> Result<(), WriteError> {
        if let Some(ref pedigree) = family_link.pedigree_linkage_type {
            self.write_value_or_wrap(level, "PEDI", Some(pedigree_to_tag(pedigree)))?;
        }

        if let Some(ref status) = family_link.child_linkage_status {
            self.write_value_or_wrap(level, "STAT", Some(child_linkage_status_to_tag(status)))?;
        }

        if let Some(ref adopted_by) = family_link.adopted_by {
            self.write_value_or_wrap(level, "ADOP", Some(adopted_by_to_tag(adopted_by)))?;
        }

        for note in &family_link.notes {
            self.write_note(level, note)?;
        }

        self.write_custom_data(level, &family_link.custom_data)?;

        Ok(())
    }

    /// Writes an association (tag: `ASSO`) — a pointer to an individual with
    /// whom this individual/event has some relationship not covered by other
    /// standard tags (e.g. a witness or godparent).
    fn write_association(
        &mut self,
        level: usize,
        association: &Association,
    ) -> Result<(), WriteError> {
        self.write_pointer(level, "ASSO", &association.xref)?;

        if self.v7() {
            if let Some(ref phrase) = association.phrase {
                self.write_value_or_wrap(level + 1, "PHRASE", Some(phrase))?;
            }
            // 7.0 requires a ROLE from its enumeration; a 5.5.1 RELA, which is
            // free text, becomes OTHER with the text as its PHRASE.
            let (role, phrase) = match (&association.role, &association.relationship) {
                (Some(role), _) => (Some(role.as_str()), association.role_phrase.as_deref()),
                (None, Some(relationship)) => (Some("OTHER"), Some(relationship.as_str())),
                (None, None) => (None, None),
            };
            if let Some(role) = role {
                self.write_line(level + 1, "ROLE", Some(role))?;
                if let Some(phrase) = phrase {
                    self.write_value_or_wrap(level + 2, "PHRASE", Some(phrase))?;
                }
            }
        } else {
            // 5.5.1 states the relationship in words (RELA). A role read from a
            // 7.0 file is the next best thing.
            let relationship = association
                .relationship
                .as_deref()
                .or(association.role_phrase.as_deref())
                .or(association.role.as_deref());
            if let Some(relationship) = relationship {
                self.write_value_or_wrap(level + 1, "RELA", Some(relationship))?;
            }
        }

        if let Some(ref association_type) = association.association_type {
            self.write_value_or_wrap(level + 1, "TYPE", Some(association_type))?;
        }
        self.write_custom_data(level + 1, &association.custom_data)?;

        for note in &association.notes {
            self.write_note(level + 1, note)?;
        }

        for citation in &association.sources {
            self.write_citation(level + 1, citation)?;
        }

        Ok(())
    }

    /// Writes a place structure.
    fn write_place(
        &mut self,
        level: usize,
        place: &crate::types::place::Place,
    ) -> Result<(), WriteError> {
        self.write_value_or_wrap(level, "PLAC", place.value.as_deref())?;
        if let Some(ref form) = place.form {
            self.write_value_or_wrap(level + 1, "FORM", Some(form))?;
        }
        if let Some(ref map) = place.map {
            self.write_line(level + 1, "MAP", None)?;
            if let Some(ref lat) = map.latitude {
                self.write_value_or_wrap(level + 2, "LATI", Some(lat))?;
            }
            if let Some(ref lon) = map.longitude {
                self.write_value_or_wrap(level + 2, "LONG", Some(lon))?;
            }
        }
        self.write_place_references(level + 1, place)?;
        for phonetic in &place.phonetic {
            self.write_value_or_wrap(level + 1, "FONE", Some(&phonetic.value))?;
            if let Some(ref vtype) = phonetic.variation_type {
                self.write_value_or_wrap(level + 2, "TYPE", Some(vtype))?;
            }
        }
        for romanized in &place.romanized {
            self.write_value_or_wrap(level + 1, "ROMN", Some(&romanized.value))?;
            if let Some(ref vtype) = romanized.variation_type {
                self.write_value_or_wrap(level + 2, "TYPE", Some(vtype))?;
            }
        }
        self.write_custom_data(level + 1, &place.custom_data)?;

        Ok(())
    }

    /// Writes what a place structure says about itself besides its name and
    /// coordinates: its `NOTE`s, the `SOUR` citations supporting it and, in
    /// GEDCOM 7.0 output, its `EXID`s. The substructures start at `level`.
    fn write_place_references(
        &mut self,
        level: usize,
        place: &crate::types::place::Place,
    ) -> Result<(), WriteError> {
        for note in &place.notes {
            self.write_note(level, note)?;
        }
        for citation in &place.citations {
            self.write_citation(level, citation)?;
        }
        if self.v7() {
            for exid in &place.external_ids {
                self.write_value_or_wrap(level, "EXID", Some(exid))?;
            }
        }
        Ok(())
    }

    /// Writes an attribute detail.
    fn write_attribute(&mut self, attr: &AttributeDetail) -> Result<(), WriteError> {
        let tag = attribute_to_tag(&attr.attribute);
        self.write_line(1, tag, attr.value.as_deref())?;

        if let Some(ref date) = attr.date {
            self.write_date(2, date)?;
        }

        if let Some(ref place) = attr.place {
            self.write_place(2, place)?;
        }

        if let Some(ref address) = attr.address {
            self.write_address(2, address)?;
        }
        self.write_contacts(2, [&attr.phone, &attr.email, &attr.fax, &attr.website])?;
        for association in &attr.associations {
            self.write_association(2, association)?;
        }

        if let Some(ref attribute_type) = attr.attribute_type {
            self.write_value_or_wrap(2, "TYPE", Some(attribute_type))?;
        }

        for citation in &attr.sources {
            self.write_citation(2, citation)?;
        }

        for media in &attr.multimedia {
            self.write_multimedia_link(2, media)?;
        }

        for note in &attr.notes {
            self.write_note(2, note)?;
        }

        if let Some(ref cause) = attr.cause {
            self.write_long_text(2, "CAUS", cause)?;
        }

        if let Some(ref restriction) = attr.restriction {
            self.write_value_or_wrap(2, "RESN", Some(restriction))?;
        }

        if let Some(ref age) = attr.age {
            self.write_age(2, age)?;
        }

        if let Some(ref agency) = attr.agency {
            self.write_value_or_wrap(2, "AGNC", Some(agency))?;
        }

        self.write_custom_data(2, &attr.custom_data)?;

        Ok(())
    }

    /// Writes the address structure of an event at `level`: its `ADDR`, then
    /// the `PHON`, `EMAIL`, `FAX` and `WWW` lines beside it.
    fn write_event_address(&mut self, level: usize, event: &EventDetail) -> Result<(), WriteError> {
        if let Some(ref address) = event.address {
            self.write_address(level, address)?;
        }
        self.write_contacts(
            level,
            [&event.phone, &event.email, &event.fax, &event.website],
        )
    }

    /// Writes the `PHON`, `EMAIL`, `FAX` and `WWW` lines of an address
    /// structure, siblings of its `ADDR`, at `level`.
    fn write_contacts(
        &mut self,
        level: usize,
        [phone, email, fax, website]: [&Vec<String>; 4],
    ) -> Result<(), WriteError> {
        for (tag, values) in [
            ("PHON", phone),
            ("EMAIL", email),
            ("FAX", fax),
            ("WWW", website),
        ] {
            for value in values {
                self.write_value_or_wrap(level, tag, Some(value))?;
            }
        }
        Ok(())
    }

    /// Writes a family record.
    fn write_family(&mut self, family: &Family) -> Result<(), WriteError> {
        self.write_record("FAM", family.xref.as_deref())?;

        if let Some(ref restriction) = family.restriction {
            self.write_value_or_wrap(1, "RESN", Some(restriction))?;
        }

        if let Some(ref husb) = family.individual1 {
            self.write_pointer(1, "HUSB", husb)?;
        }

        if let Some(ref wife) = family.individual2 {
            self.write_pointer(1, "WIFE", wife)?;
        }

        for child in &family.children {
            self.write_pointer(1, "CHIL", child)?;
        }

        for submitter in &family.submitters {
            self.write_pointer(1, "SUBM", submitter)?;
        }

        for event in &family.events {
            self.write_event(1, event)?;
        }

        // GEDCOM 7.0: Non-events
        for non_event in &family.non_events {
            self.write_non_event(1, non_event)?;
        }

        // LDS Sealing to Spouse (SLGS)
        for ordinance in &family.lds_ordinances {
            self.write_lds_ordinance(1, ordinance)?;
        }

        if let Some(ref num_children) = family.num_children {
            self.write_value_or_wrap(1, "NCHI", Some(num_children))?;
        }
        self.write_record_identifiers(
            family.user_reference_number.as_deref(),
            family.user_reference_type.as_deref(),
            family.automated_record_id.as_deref(),
            family.uid.as_deref(),
            &family.external_ids,
        )?;

        for citation in &family.sources {
            self.write_citation(1, citation)?;
        }

        for media in &family.multimedia {
            self.write_multimedia_link(1, media)?;
        }

        for note in &family.notes {
            self.write_note(1, note)?;
        }

        if let Some(ref change_date) = family.change_date {
            self.write_line(1, "CHAN", None)?;
            if let Some(ref date) = change_date.date {
                self.write_date(2, date)?;
            }
        }

        self.write_custom_data(1, &family.custom_data)?;

        Ok(())
    }

    /// Writes the identifiers of an individual or family record: `REFN` with
    /// its `TYPE`, `RIN`, and the GEDCOM 7.0 `UID` and `EXID`, which 5.5.1 does
    /// not define.
    fn write_record_identifiers(
        &mut self,
        refn: Option<&str>,
        refn_type: Option<&str>,
        rin: Option<&str>,
        uid: Option<&str>,
        external_ids: &[String],
    ) -> Result<(), WriteError> {
        if let Some(refn) = refn {
            self.write_value_or_wrap(1, "REFN", Some(refn))?;
            if let Some(refn_type) = refn_type {
                self.write_value_or_wrap(2, "TYPE", Some(refn_type))?;
            }
        }
        if let Some(rin) = rin {
            self.write_value_or_wrap(1, "RIN", Some(rin))?;
        }
        if self.v7() {
            if let Some(uid) = uid {
                self.write_value_or_wrap(1, "UID", Some(uid))?;
            }
            for exid in external_ids {
                self.write_value_or_wrap(1, "EXID", Some(exid))?;
            }
        }
        Ok(())
    }

    /// Writes a source record.
    fn write_source(&mut self, source: &Source) -> Result<(), WriteError> {
        self.write_record("SOUR", source.xref.as_deref())?;

        // What the source records: `DATA` with its `EVEN`, `AGNC` and `NOTE`
        if !source.data.is_empty() {
            self.write_source_data(1, &source.data)?;
        }

        if let Some(ref title) = source.title {
            self.write_long_text(1, "TITL", title)?;
        }

        if let Some(ref author) = source.author {
            self.write_long_text(1, "AUTH", author)?;
        }

        if let Some(ref abbr) = source.abbreviation {
            self.write_value_or_wrap(1, "ABBR", Some(abbr))?;
        }

        if let Some(ref publication) = source.publication_facts {
            self.write_long_text(1, "PUBL", publication)?;
        }

        if let Some(ref text) = source.citation_from_source {
            self.write_long_text(1, "TEXT", text)?;
        }

        // Repository citations
        for repo in &source.repo_citations {
            self.write_repository_citation(1, repo)?;
        }

        // Record identifiers
        if let Some(ref refn) = source.user_reference_number {
            self.write_value_or_wrap(1, "REFN", Some(refn))?;
            if let Some(ref refn_type) = source.user_reference_type {
                self.write_value_or_wrap(2, "TYPE", Some(refn_type))?;
            }
        }
        if let Some(ref rin) = source.automated_record_id {
            self.write_value_or_wrap(1, "RIN", Some(rin))?;
        }
        if let Some(ref rfn) = source.submitter_registered_rfn {
            self.write_value_or_wrap(1, "RFN", Some(rfn))?;
        }
        if self.v7() {
            if let Some(ref uid) = source.uid {
                self.write_value_or_wrap(1, "UID", Some(uid))?;
            }
            for exid in &source.external_ids {
                self.write_value_or_wrap(1, "EXID", Some(exid))?;
            }
        }

        // Notes
        for note in &source.notes {
            self.write_note(1, note)?;
        }

        // Multimedia links
        for media in &source.multimedia {
            self.write_multimedia_link(1, media)?;
        }

        // Change date
        if let Some(ref change_date) = source.change_date {
            self.write_line(1, "CHAN", None)?;
            if let Some(ref date) = change_date.date {
                self.write_date(2, date)?;
            }
        }

        self.write_custom_data(1, &source.custom_data)?;

        Ok(())
    }

    /// Writes the `DATA` substructure of a source record.
    fn write_source_data(&mut self, level: usize, data: &SourceData) -> Result<(), WriteError> {
        self.write_line(level, "DATA", None)?;

        for event in data.events() {
            let recorded = match event.event {
                Event::SourceData(ref recorded) => Some(recorded.as_str()),
                _ => event.value.as_deref(),
            };
            self.write_value_or_wrap(level + 1, "EVEN", recorded)?;
            if let Some(ref date) = event.date {
                self.write_date(level + 2, date)?;
            }
            if let Some(ref place) = event.place {
                self.write_value_or_wrap(level + 2, "PLAC", place.value.as_deref())?;
            }
        }

        if let Some(ref agency) = data.agency {
            self.write_value_or_wrap(level + 1, "AGNC", Some(agency))?;
        }

        for note in &data.notes {
            self.write_note(level + 1, note)?;
        }

        Ok(())
    }

    /// Writes a repository record.
    fn write_repository(&mut self, repo: &Repository) -> Result<(), WriteError> {
        self.write_record("REPO", repo.xref.as_deref())?;

        if let Some(ref name) = repo.name {
            self.write_value_or_wrap(1, "NAME", Some(name))?;
        }
        self.write_custom_data(1, &repo.custom_data)?;

        if let Some(ref address) = repo.address {
            self.write_address(1, address)?;
        }

        // The rest of the 5.5.1 ADDRESS_STRUCTURE, siblings of ADDR
        for (tag, values) in [
            ("PHON", &repo.phone),
            ("EMAIL", &repo.email),
            ("FAX", &repo.fax),
            ("WWW", &repo.website),
        ] {
            for value in values {
                self.write_value_or_wrap(1, tag, Some(value))?;
            }
        }

        for note in &repo.notes {
            self.write_note(1, note)?;
        }

        if let Some(ref refn) = repo.user_reference_number {
            self.write_value_or_wrap(1, "REFN", Some(refn))?;
            if let Some(ref refn_type) = repo.user_reference_type {
                self.write_value_or_wrap(2, "TYPE", Some(refn_type))?;
            }
        }

        if let Some(ref rin) = repo.automated_record_id {
            self.write_value_or_wrap(1, "RIN", Some(rin))?;
        }

        // UID and EXID exist in GEDCOM 7.0 only
        if self.v7() {
            if let Some(ref uid) = repo.uid {
                self.write_value_or_wrap(1, "UID", Some(uid))?;
            }
            for exid in &repo.external_ids {
                self.write_value_or_wrap(1, "EXID", Some(exid))?;
            }
        }

        if let Some(ref change_date) = repo.change_date {
            self.write_line(1, "CHAN", None)?;
            if let Some(ref date) = change_date.date {
                self.write_date(2, date)?;
            }
        }

        Ok(())
    }

    /// Writes a submitter record.
    fn write_submitter(&mut self, submitter: &Submitter) -> Result<(), WriteError> {
        self.write_record("SUBM", submitter.xref.as_deref())?;

        if let Some(ref name) = submitter.name {
            self.write_value_or_wrap(1, "NAME", Some(name))?;
        }

        if let Some(ref address) = submitter.address {
            self.write_address(1, address)?;
        }

        // The rest of the 5.5.1 ADDRESS_STRUCTURE, siblings of ADDR
        for (tag, values) in [
            ("PHON", &submitter.phone),
            ("EMAIL", &submitter.email),
            ("FAX", &submitter.fax),
            ("WWW", &submitter.website),
        ] {
            for value in values {
                self.write_value_or_wrap(1, tag, Some(value))?;
            }
        }

        for link in &submitter.multimedia {
            self.write_submitter_multimedia(link)?;
        }

        if let Some(ref lang) = submitter.language {
            self.write_value_or_wrap(1, "LANG", Some(lang))?;
        }

        let gedcom_5 = !self.v7();
        // RFN (Ancestral File) was removed in GEDCOM 7.0; UID was added.
        if let Some(ref rfn) = submitter.registered_refn {
            if gedcom_5 {
                self.write_value_or_wrap(1, "RFN", Some(rfn))?;
            }
        }
        if let Some(ref refn) = submitter.user_reference_number {
            self.write_value_or_wrap(1, "REFN", Some(refn))?;
        }
        if let Some(ref rin) = submitter.automated_record_id {
            self.write_value_or_wrap(1, "RIN", Some(rin))?;
        }
        if let Some(ref uid) = submitter.uid {
            if !gedcom_5 {
                self.write_value_or_wrap(1, "UID", Some(uid))?;
            }
        }

        // Note
        for note in &submitter.notes {
            self.write_note(1, note)?;
        }

        // Change date
        if let Some(ref change_date) = submitter.change_date {
            self.write_line(1, "CHAN", None)?;
            if let Some(ref date) = change_date.date {
                self.write_date(2, date)?;
            }
        }

        self.write_custom_data(1, &submitter.custom_data)?;

        Ok(())
    }

    /// Writes a submitter's multimedia link: a pointer to a multimedia record,
    /// or the file it describes inline.
    fn write_submitter_multimedia(
        &mut self,
        link: &crate::types::multimedia::link::Link,
    ) -> Result<(), WriteError> {
        if let Some(ref xref) = link.xref {
            return self.write_pointer(1, "OBJE", xref);
        }
        self.write_line(1, "OBJE", None)?;
        if let Some(ref file) = link.file {
            self.write_value_or_wrap(2, "FILE", file.value.as_deref())?;
            if let Some(ref format) = file.form {
                self.write_value_or_wrap(3, "FORM", format.value.as_deref())?;
                if let Some(ref media_type) = format.source_media_type {
                    self.write_value_or_wrap(4, "TYPE", Some(media_type))?;
                }
            }
            if let Some(ref title) = file.title {
                self.write_value_or_wrap(3, "TITL", Some(title))?;
            }
        }
        if let Some(ref form) = link.form {
            self.write_value_or_wrap(2, "FORM", form.value.as_deref())?;
            if let Some(ref media_type) = form.source_media_type {
                self.write_value_or_wrap(3, "TYPE", Some(media_type))?;
            }
        }
        if let Some(ref title) = link.title {
            self.write_value_or_wrap(2, "TITL", Some(title))?;
        }
        Ok(())
    }

    /// Writes a submission record.
    fn write_submission(&mut self, submission: &Submission) -> Result<(), WriteError> {
        self.write_record("SUBN", submission.xref.as_deref())?;

        if let Some(ref subm) = submission.submitter_ref {
            self.write_pointer(1, "SUBM", subm)?;
        }

        if let Some(ref file) = submission.family_file_name {
            self.write_value_or_wrap(1, "FAMF", Some(file))?;
        }

        if let Some(ref temple) = submission.temple_code {
            self.write_value_or_wrap(1, "TEMP", Some(temple))?;
        }

        if let Some(ref ancestors) = submission.ancestor_generations {
            self.write_value_or_wrap(1, "ANCE", Some(ancestors))?;
        }

        if let Some(ref descendants) = submission.descendant_generations {
            self.write_value_or_wrap(1, "DESC", Some(descendants))?;
        }

        for note in &submission.notes {
            self.write_note(1, note)?;
        }

        Ok(())
    }

    /// Writes a multimedia record.
    fn write_multimedia(&mut self, media: &Multimedia) -> Result<(), WriteError> {
        self.write_record("OBJE", media.xref.as_deref())?;
        self.write_multimedia_substructures(1, media)?;

        // User reference number
        if let Some(ref refn) = media.user_reference_number {
            self.write_value_or_wrap(1, "REFN", refn.value.as_deref())?;
            if let Some(ref refn_type) = refn.user_reference_type {
                self.write_value_or_wrap(2, "TYPE", Some(refn_type))?;
            }
        }

        // Automated record ID
        if let Some(ref rin) = media.automated_record_id {
            self.write_value_or_wrap(1, "RIN", Some(rin))?;
        }

        // Note
        for note in &media.notes {
            self.write_note(1, note)?;
        }

        // Source citation
        if let Some(ref citation) = media.source_citation {
            self.write_citation(1, citation)?;
        }

        // Change date
        if let Some(ref change_date) = media.change_date {
            self.write_line(1, "CHAN", None)?;
            if let Some(ref date) = change_date.date {
                self.write_date(2, date)?;
            }
        }

        self.write_custom_data(1, &media.custom_data)?;

        Ok(())
    }

    /// Writes a source's repository citation: the `REPO` pointer with its
    /// `NOTE`s and its call numbers, each `CALN` with its own `MEDI`.
    fn write_repository_citation(
        &mut self,
        level: usize,
        repo: &crate::types::repository::citation::Citation,
    ) -> Result<(), WriteError> {
        self.write_pointer(level, "REPO", &repo.xref)?;

        for note in &repo.notes {
            self.write_note(level + 1, note)?;
        }

        let gedcom_5 = !self.v7();
        for call_number in &repo.call_numbers {
            self.write_value_or_wrap(level + 1, "CALN", Some(&call_number.value))?;
            let Some(ref medium) = call_number.medium else {
                continue;
            };
            let phrase = call_number.medium_phrase.as_deref();
            let standard = SOURCE_MEDIA_TYPES
                .iter()
                .find(|known| known.eq_ignore_ascii_case(medium));
            if gedcom_5 {
                // 5.5.1 values are lower case and it has no PHRASE: the text of
                // an `OTHER` medium is the best value available.
                let value = match (standard, phrase) {
                    (Some(known), _) => known.to_string(),
                    (None, Some(phrase)) if medium.eq_ignore_ascii_case("OTHER") => {
                        phrase.to_string()
                    }
                    (None, _) => medium.clone(),
                };
                self.write_value_or_wrap(level + 2, "MEDI", Some(&value))?;
            } else {
                // 7.0 values are an upper-case enumeration; anything else is
                // `OTHER` with the text as its PHRASE.
                let (value, phrase) = match standard {
                    Some(known) => (known.to_ascii_uppercase(), phrase),
                    None if medium.eq_ignore_ascii_case("OTHER") => ("OTHER".to_string(), phrase),
                    None => ("OTHER".to_string(), Some(phrase.unwrap_or(medium))),
                };
                self.write_line(level + 2, "MEDI", Some(&value))?;
                if let Some(phrase) = phrase {
                    self.write_value_or_wrap(level + 3, "PHRASE", Some(phrase))?;
                }
            }
        }

        Ok(())
    }

    /// Writes a multimedia link (embedded reference).
    fn write_multimedia_link(
        &mut self,
        level: usize,
        media: &Multimedia,
    ) -> Result<(), WriteError> {
        if let Some(ref xref) = media.xref {
            self.write_pointer(level, "OBJE", xref)?;
        } else {
            self.write_line(level, "OBJE", None)?;
            self.write_multimedia_substructures(level + 1, media)?;
        }
        self.write_custom_data(level + 1, &media.custom_data)?;
        Ok(())
    }

    /// Writes the `FILE`, `FORM` and `TITL` substructures shared by a multimedia
    /// record and an inline multimedia link, starting at `level`.
    fn write_multimedia_substructures(
        &mut self,
        level: usize,
        media: &Multimedia,
    ) -> Result<(), WriteError> {
        if let Some(ref file) = media.file {
            self.write_value_or_wrap(level, "FILE", file.value.as_deref())?;
            if let Some(ref format) = file.form {
                self.write_value_or_wrap(level + 1, "FORM", format.value.as_deref())?;
                if let Some(ref media_type) = format.source_media_type {
                    self.write_value_or_wrap(level + 2, "TYPE", Some(media_type))?;
                }
            }
            if let Some(ref title) = file.title {
                self.write_value_or_wrap(level + 1, "TITL", Some(title))?;
            }
            if let Some(ref crop) = file.crop {
                self.write_crop(level + 1, crop)?;
            }
        }

        // The 5.5 spec puts FORM and TITL under FILE, but some exporters
        // (e.g. Ancestry.com) write them as siblings of it.
        if let Some(ref form) = media.form {
            self.write_line(level, "FORM", form.value.as_deref())?;
            if let Some(ref media_type) = form.source_media_type {
                self.write_value_or_wrap(level + 1, "TYPE", Some(media_type))?;
            }
        }

        if let Some(ref title) = media.title {
            self.write_value_or_wrap(level, "TITL", Some(title))?;
        }

        Ok(())
    }

    /// Writes an image cropping region (GEDCOM 7.0).
    fn write_crop(&mut self, level: usize, crop: &Crop) -> Result<(), WriteError> {
        self.write_line(level, "CROP", None)?;

        for (tag, value) in [
            ("TOP", crop.top),
            ("LEFT", crop.left),
            ("HEIGHT", crop.height),
            ("WIDTH", crop.width),
        ] {
            if let Some(value) = value {
                self.write_line(level + 1, tag, Some(&value.to_string()))?;
            }
        }

        Ok(())
    }

    /// Writes a source citation.
    fn write_citation(&mut self, level: usize, citation: &Citation) -> Result<(), WriteError> {
        match citation.source {
            CitationSource::Xref(ref xref) => self.write_pointer(level, "SOUR", xref)?,
            // A free-text description may span several lines.
            CitationSource::Description(ref text) => self.write_long_text(level, "SOUR", text)?,
        }

        if let Some(ref page) = citation.page {
            self.write_value_or_wrap(level + 1, "PAGE", Some(page))?;
        }

        if let Some(ref event_type) = citation.event_type {
            self.write_value_or_wrap(level + 1, "EVEN", Some(event_type))?;
            if let Some(ref role) = citation.role {
                self.write_value_or_wrap(level + 2, "ROLE", Some(role))?;
            }
        }

        if let Some(ref data) = citation.data {
            self.write_line(level + 1, "DATA", None)?;
            self.write_custom_data(level + 2, &data.custom_data)?;
            if let Some(ref date) = data.date {
                self.write_date(level + 2, date)?;
            }
            for text in &data.texts {
                if let Some(ref text_value) = text.value {
                    self.write_long_text(level + 2, "TEXT", text_value)?;
                }
            }
        }

        for text in &citation.texts {
            if let Some(ref text_value) = text.value {
                self.write_long_text(level + 1, "TEXT", text_value)?;
            }
        }

        for media in &citation.multimedia {
            self.write_multimedia_link(level + 1, media)?;
        }

        if let Some(ref rfn) = citation.submitter_registered_rfn {
            self.write_value_or_wrap(level + 1, "RFN", Some(rfn))?;
        }

        if let Some(ref certainty) = citation.certainty_assessment {
            if let Some(quay) = certainty_to_gedcom_value(certainty) {
                self.write_line(level + 1, "QUAY", Some(quay))?;
            }
        }

        for note in &citation.notes {
            self.write_note(level + 1, note)?;
        }

        self.write_custom_data(level + 1, &citation.custom_data)?;

        Ok(())
    }

    /// Writes a date structure, converted by [`Date::to_version`] to the
    /// grammar of the target version. GEDCOM 5.5.1 has no `PHRASE`: a phrase
    /// that cannot move into the payload (next to a range or a period) is
    /// left out.
    fn write_date(&mut self, level: usize, date: &Date) -> Result<(), WriteError> {
        let version = self.rules.version;
        let gedcom_7 = version.is_v7();
        let date = date.to_version(version);
        let value = date
            .value
            .as_deref()
            .map(str::trim)
            .filter(|v| !v.is_empty());
        let phrase = date.phrase.as_deref().filter(|p| !p.is_empty() && gedcom_7);
        // A TIME or PHRASE without its DATE would belong to the parent.
        let payload = match (value, phrase) {
            (Some(value), _) => value,
            (None, Some(_)) => "",
            (None, None) => return Ok(()),
        };

        self.write_value_or_wrap(level, "DATE", Some(payload))?;

        if let Some(ref time) = date.time {
            self.write_value_or_wrap(level + 1, "TIME", Some(time))?;
        }

        if let Some(phrase) = phrase {
            self.write_value_or_wrap(level + 1, "PHRASE", Some(phrase))?;
        }

        Ok(())
    }

    /// Writes a schema structure (GEDCOM 7.0).
    fn write_schema(&mut self, schema: &Schema) -> Result<(), WriteError> {
        self.write_line(1, "SCHMA", None)?;

        for tag_def in &schema.tag_definitions {
            let payload = tag_def.to_payload();
            self.write_value_or_wrap(2, "TAG", Some(&payload))?;
        }

        self.write_custom_data(2, &schema.custom_data)?;

        Ok(())
    }

    /// Writes a shared note record (GEDCOM 7.0).
    fn write_shared_note(&mut self, note: &SharedNote) -> Result<(), WriteError> {
        // The text spans as many CONT/CONC lines as it needs, like any other
        // long text; written raw, its newlines would start bogus lines.
        let tag = if self.v7() { "SNOTE" } else { "NOTE" };
        self.write_record_text(tag, note.xref.as_deref(), &note.text)?;

        if let Some(ref mime) = note.mime {
            self.write_value_or_wrap(1, "MIME", Some(mime))?;
        }

        if let Some(ref lang) = note.language {
            self.write_value_or_wrap(1, "LANG", Some(lang))?;
        }

        for translation in &note.translations {
            self.write_value_or_wrap(1, "TRAN", Some(&translation.text))?;
            if let Some(ref mime) = translation.mime {
                self.write_value_or_wrap(2, "MIME", Some(mime))?;
            }
            if let Some(ref lang) = translation.language {
                self.write_value_or_wrap(2, "LANG", Some(lang))?;
            }
        }

        for exid in &note.external_ids {
            self.write_value_or_wrap(1, "EXID", Some(&exid.id))?;
            if let Some(ref type_uri) = exid.type_uri {
                self.write_value_or_wrap(2, "TYPE", Some(type_uri))?;
            }
        }

        for citation in &note.source_citations {
            self.write_citation(1, citation)?;
        }

        self.write_custom_data(1, &note.custom_data)?;

        Ok(())
    }

    /// Writes a sort date structure (GEDCOM 7.0).
    fn write_sort_date(&mut self, level: usize, sort_date: &SortDate) -> Result<(), WriteError> {
        if let Some(ref value) = sort_date.value {
            self.write_value_or_wrap(level, "SDATE", Some(value))?;
        }

        if let Some(ref time) = sort_date.time {
            self.write_value_or_wrap(level + 1, "TIME", Some(time))?;
        }

        // PHRASE does not exist in GEDCOM 5.5.1.
        if let Some(ref phrase) = sort_date.phrase {
            if self.v7() {
                self.write_value_or_wrap(level + 1, "PHRASE", Some(phrase))?;
            }
        }

        Ok(())
    }

    /// Writes a non-event structure (GEDCOM 7.0).
    fn write_non_event(&mut self, level: usize, non_event: &NonEvent) -> Result<(), WriteError> {
        self.write_line(level, "NO", Some(&non_event.event_type))?;

        if let Some(ref date) = non_event.date {
            self.write_date(level + 1, date)?;
        }

        for note in &non_event.notes {
            self.write_note(level + 1, note)?;
        }

        for citation in &non_event.source_citations {
            self.write_citation(level + 1, citation)?;
        }

        Ok(())
    }

    /// Writes an LDS ordinance structure.
    fn write_lds_ordinance(
        &mut self,
        level: usize,
        ordinance: &LdsOrdinance,
    ) -> Result<(), WriteError> {
        let tag = ordinance
            .ordinance_type
            .as_ref()
            .map_or("BAPL", |t| t.to_tag());

        self.write_line(level, tag, None)?;

        if let Some(ref date) = ordinance.date {
            self.write_date(level + 1, date)?;
        }

        if let Some(ref temple) = ordinance.temple {
            self.write_value_or_wrap(level + 1, "TEMP", Some(temple))?;
        }

        if let Some(ref place) = ordinance.place {
            self.write_place(level + 1, place)?;
        }

        if let Some(ref status) = ordinance.status {
            self.write_line(level + 1, "STAT", Some(status.to_gedcom_value()))?;

            if let Some(ref status_date) = ordinance.status_date {
                self.write_date(level + 2, status_date)?;
            }
        }

        if let Some(ref famc) = ordinance.family_xref {
            self.write_pointer(level + 1, "FAMC", famc)?;
        }

        for note in &ordinance.notes {
            self.write_note(level + 1, note)?;
        }

        for citation in &ordinance.source_citations {
            self.write_citation(level + 1, citation)?;
        }

        Ok(())
    }

    /// Writes an address structure.
    fn write_address(&mut self, level: usize, address: &Address) -> Result<(), WriteError> {
        self.write_value_or_wrap(level, "ADDR", address.value.as_deref())?;

        if let Some(ref line1) = address.adr1 {
            self.write_value_or_wrap(level + 1, "ADR1", Some(line1))?;
        }

        if let Some(ref line2) = address.adr2 {
            self.write_value_or_wrap(level + 1, "ADR2", Some(line2))?;
        }

        if let Some(ref line3) = address.adr3 {
            self.write_value_or_wrap(level + 1, "ADR3", Some(line3))?;
        }

        if let Some(ref city) = address.city {
            self.write_value_or_wrap(level + 1, "CITY", Some(city))?;
        }

        if let Some(ref state) = address.state {
            self.write_value_or_wrap(level + 1, "STAE", Some(state))?;
        }

        if let Some(ref postal) = address.post {
            self.write_value_or_wrap(level + 1, "POST", Some(postal))?;
        }

        if let Some(ref country) = address.country {
            self.write_value_or_wrap(level + 1, "CTRY", Some(country))?;
        }

        self.write_custom_data(level + 1, &address.custom_data)?;

        Ok(())
    }

    /// Writes a note structure.
    fn write_note(&mut self, level: usize, note: &Note) -> Result<(), WriteError> {
        if let Some(xref) = note.shared_note_xref() {
            // A pointer to a shared note record: `NOTE @N1@` in 5.5.1,
            // `SNOTE @N1@` in 7.0, where a NOTE payload is always text.
            let tag = if self.v7() { "SNOTE" } else { "NOTE" };
            self.write_pointer(level, tag, xref)?;
        } else if let Some(ref value) = note.value {
            self.write_long_text(level, "NOTE", value)?;
        } else {
            self.write_line(level, "NOTE", None)?;
        }

        Ok(())
    }

    /// Writes extension (user-defined) tags, and the unknown tags kept the
    /// same way, each with its substructures, the first ones at `level`
    /// (records when `level` is 0). Their values are untyped: one with the
    /// shape of a pointer is written as a pointer, any other as text.
    fn write_custom_data(
        &mut self,
        level: usize,
        tags: &[Box<UserDefinedTag>],
    ) -> Result<(), WriteError> {
        for tag in tags {
            if level == 0 {
                let name = match tag.tag.as_str() {
                    // Only the writer writes the header and the trailer.
                    "HEAD" | "TRLR" => Cow::Owned(format!("_{}", tag.tag)),
                    other => Cow::Borrowed(other),
                };
                let index = self.next_record();
                let xref = self
                    .xrefs
                    .record(index, tag.xref.as_deref())
                    .map(str::to_string);
                self.put_untyped(0, xref.as_deref(), &name, tag.value.as_deref())?;
            } else {
                self.put_untyped(level, tag.xref.as_deref(), &tag.tag, tag.value.as_deref())?;
            }
            // The substructures, depth first, without recursion.
            let mut stack = vec![(level + 1, tag.children.iter())];
            while let Some((child_level, children)) = stack.last_mut() {
                let child_level = *child_level;
                let Some(child) = children.next() else {
                    stack.pop();
                    continue;
                };
                self.put_untyped(
                    child_level,
                    child.xref.as_deref(),
                    &child.tag,
                    child.value.as_deref(),
                )?;
                stack.push((child_level + 1, child.children.iter()));
            }
        }
        Ok(())
    }
}

/// Whether a pointer field's value has the shape of a pointer, `@…@`
/// (surrounding spaces aside); anything else is text kept in a pointer field.
fn is_pointer_shaped(value: &str) -> bool {
    let v = value.trim();
    v.len() >= 3 && v.starts_with('@') && v.ends_with('@') && !v[1..v.len() - 1].contains('@')
}

// =============================================================================
// Helper functions for tag conversion
// =============================================================================

/// The `SOURCE_MEDIA_TYPE` values of GEDCOM 5.5.1, which GEDCOM 7.0 writes in
/// upper case (and extends with `OTHER`).
const SOURCE_MEDIA_TYPES: [&str; 13] = [
    "audio",
    "book",
    "card",
    "electronic",
    "fiche",
    "film",
    "magazine",
    "manuscript",
    "map",
    "newspaper",
    "photo",
    "tombstone",
    "video",
];

/// Converts an event type to its GEDCOM tag.
fn event_to_tag(event: &Event) -> &'static str {
    match event {
        Event::Adoption => "ADOP",
        Event::Birth => "BIRT",
        Event::Baptism => "BAPM",
        Event::BarMitzvah => "BARM",
        Event::BasMitzvah => "BASM",
        Event::Blessing => "BLES",
        Event::Burial => "BURI",
        Event::Census => "CENS",
        Event::Christening => "CHR",
        Event::AdultChristening => "CHRA",
        Event::Confirmation => "CONF",
        Event::Cremation => "CREM",
        Event::Death => "DEAT",
        Event::Emigration => "EMIG",
        Event::FirstCommunion => "FCOM",
        Event::Graduation => "GRAD",
        Event::Immigration => "IMMI",
        Event::Naturalization => "NATU",
        Event::Ordination => "ORDN",
        Event::Retired => "RETI",
        Event::Probate => "PROB",
        Event::Will => "WILL",
        Event::Marriage => "MARR",
        Event::Annulment => "ANUL",
        Event::Divorce => "DIV",
        Event::DivorceFiled => "DIVF",
        Event::Engagement => "ENGA",
        Event::MarriageBann => "MARB",
        Event::MarriageContract => "MARC",
        Event::MarriageLicense => "MARL",
        Event::MarriageSettlement => "MARS",
        Event::Residence => "RESI",
        Event::Separated => "SEP",
        Event::Event | Event::Other => "EVEN",
        Event::SourceData(_) => "DATA",
    }
}

/// Converts an individual attribute type to its GEDCOM tag.
fn attribute_to_tag(
    attr: &crate::types::individual::attribute::IndividualAttribute,
) -> &'static str {
    use crate::types::individual::attribute::IndividualAttribute;
    match attr {
        IndividualAttribute::CastName => "CAST",
        IndividualAttribute::PhysicalDescription => "DSCR",
        IndividualAttribute::ScholasticAchievement => "EDUC",
        IndividualAttribute::NationalIDNumber => "IDNO",
        IndividualAttribute::NationalOrTribalOrigin => "NATI",
        IndividualAttribute::CountOfChildren => "NCHI",
        IndividualAttribute::CountOfMarriages => "NMR",
        IndividualAttribute::Occupation => "OCCU",
        IndividualAttribute::Possessions => "PROP",
        IndividualAttribute::ReligiousAffiliation => "RELI",
        IndividualAttribute::ResidesAt => "RESI",
        IndividualAttribute::SocialSecurityNumber => "SSN",
        IndividualAttribute::NobilityTypeTitle => "TITL",
        IndividualAttribute::Fact => "FACT",
    }
}

/// Converts a certainty assessment to its GEDCOM value.
fn certainty_to_gedcom_value(certainty: &CertaintyAssessment) -> Option<&'static str> {
    match certainty {
        CertaintyAssessment::Unreliable => Some("0"),
        CertaintyAssessment::Questionable => Some("1"),
        CertaintyAssessment::Secondary => Some("2"),
        CertaintyAssessment::Direct => Some("3"),
        CertaintyAssessment::None => None,
    }
}

/// Converts a `PEDI` pedigree code to its GEDCOM 5.5.1 value.
fn pedigree_to_tag(
    pedigree: &crate::types::individual::family_link::pedigree::Pedigree,
) -> &'static str {
    use crate::types::individual::family_link::pedigree::Pedigree;
    match pedigree {
        Pedigree::Adopted => "adopted",
        Pedigree::Birth => "birth",
        Pedigree::Foster => "foster",
        Pedigree::Sealing => "sealing",
    }
}

/// Converts a `FAMC.STAT` child linkage status to its GEDCOM 5.5.1 value.
fn child_linkage_status_to_tag(
    status: &crate::types::individual::family_link::child_link::ChildLinkStatus,
) -> &'static str {
    use crate::types::individual::family_link::child_link::ChildLinkStatus;
    match status {
        ChildLinkStatus::Challenged => "challenged",
        ChildLinkStatus::Disproven => "disproven",
        ChildLinkStatus::Proven => "proven",
    }
}

/// Converts an `ADOP` (adopted-by-which-parent) code to its GEDCOM 5.5.1 value.
fn adopted_by_to_tag(
    adopted_by: &crate::types::individual::family_link::adopted::AdoptedByWhichParent,
) -> &'static str {
    use crate::types::individual::family_link::adopted::AdoptedByWhichParent;
    match adopted_by {
        AdoptedByWhichParent::Husband => "HUSB",
        AdoptedByWhichParent::Wife => "WIFE",
        AdoptedByWhichParent::Both => "BOTH",
    }
}

// =============================================================================
// Helper trait implementation for family link type
// =============================================================================

impl crate::types::individual::family_link::FamilyLinkType {
    /// Converts a family link type to its GEDCOM tag.
    fn to_tag(&self) -> &'static str {
        use crate::types::individual::family_link::FamilyLinkType;
        match self {
            FamilyLinkType::Child => "FAMC",
            FamilyLinkType::Spouse => "FAMS",
        }
    }
}

#[cfg(test)]
mod tests {
    use crate::writer::LineEnding;
    use crate::{GedcomBuilder, GedcomVersion, GedcomWriter};

    #[test]
    fn test_write_minimal_gedcom() {
        let source = "0 HEAD\n1 GEDC\n2 VERS 5.5\n0 TRLR";
        let data = GedcomBuilder::new().build_from_str(source).unwrap();

        let writer = GedcomWriter::new();
        let output = writer.write_to_string(&data).unwrap();

        assert!(output.contains("0 HEAD"));
        assert!(output.contains("1 GEDC"));
        assert!(output.contains("2 VERS"));
        assert!(output.contains("0 TRLR"));
    }

    #[test]
    fn test_write_individual() {
        let source = "0 HEAD\n1 GEDC\n2 VERS 5.5\n0 @I1@ INDI\n1 NAME John /Doe/\n1 SEX M\n0 TRLR";
        let data = GedcomBuilder::new().build_from_str(source).unwrap();

        let writer = GedcomWriter::new();
        let output = writer.write_to_string(&data).unwrap();

        assert!(output.contains("0 @I1@ INDI"));
        assert!(output.contains("1 NAME John /Doe/"));
        assert!(output.contains("1 SEX M"));
    }

    #[test]
    fn test_write_family() {
        let source =
            "0 HEAD\n1 GEDC\n2 VERS 5.5\n0 @F1@ FAM\n1 HUSB @I1@\n1 WIFE @I2@\n1 CHIL @I3@\n0 TRLR";
        let data = GedcomBuilder::new().build_from_str(source).unwrap();

        let writer = GedcomWriter::new();
        let output = writer.write_to_string(&data).unwrap();

        assert!(output.contains("0 @F1@ FAM"));
        assert!(output.contains("1 HUSB @I1@"));
        assert!(output.contains("1 WIFE @I2@"));
        assert!(output.contains("1 CHIL @I3@"));
    }

    #[test]
    fn test_write_events() {
        let source = "0 HEAD\n1 GEDC\n2 VERS 5.5\n0 @I1@ INDI\n1 NAME Test /Person/\n1 BIRT\n2 DATE 1 JAN 1900\n2 PLAC Test City\n0 TRLR";
        let data = GedcomBuilder::new().build_from_str(source).unwrap();

        let writer = GedcomWriter::new();
        let output = writer.write_to_string(&data).unwrap();

        assert!(output.contains("1 BIRT"));
        assert!(output.contains("2 DATE 1 JAN 1900"));
        assert!(output.contains("2 PLAC Test City"));
    }

    #[test]
    fn test_write_source_record() {
        let source = "0 HEAD\n1 GEDC\n2 VERS 5.5\n0 @S1@ SOUR\n1 TITL Test Source\n1 AUTH Test Author\n0 TRLR";
        let data = GedcomBuilder::new().build_from_str(source).unwrap();

        let writer = GedcomWriter::new();
        let output = writer.write_to_string(&data).unwrap();

        assert!(output.contains("0 @S1@ SOUR"));
        assert!(output.contains("1 TITL Test Source"));
        assert!(output.contains("1 AUTH Test Author"));
    }

    #[test]
    fn test_custom_line_ending() {
        let source = "0 HEAD\n1 GEDC\n2 VERS 5.5\n0 TRLR";
        let data = GedcomBuilder::new().build_from_str(source).unwrap();

        let writer = GedcomWriter::new().line_ending(LineEnding::CrLf);
        let output = writer.write_to_string(&data).unwrap();

        assert!(output.contains("\r\n"));
    }

    #[test]
    fn test_round_trip_basic() {
        let original =
            "0 HEAD\n1 GEDC\n2 VERS 5.5\n0 @I1@ INDI\n1 NAME John /Doe/\n1 SEX M\n0 TRLR";
        let data = GedcomBuilder::new().build_from_str(original).unwrap();

        let writer = GedcomWriter::new();
        let written = writer.write_to_string(&data).unwrap();

        // Parse the written output
        let data2 = GedcomBuilder::new().build_from_str(&written).unwrap();

        // Compare key data
        assert_eq!(data.individuals.len(), data2.individuals.len());
        assert_eq!(data.individuals[0].xref, data2.individuals[0].xref);
        assert_eq!(data.individuals[0].names, data2.individuals[0].names);
    }

    #[test]
    fn test_write_name_nickname() {
        let source =
            "0 HEAD\n1 GEDC\n2 VERS 5.5\n0 @I1@ INDI\n1 NAME John /Doe/\n2 NICK Johnny\n0 TRLR";
        let data = GedcomBuilder::new().build_from_str(source).unwrap();

        let writer = GedcomWriter::new();
        let output = writer.write_to_string(&data).unwrap();

        assert!(output.contains("2 NICK Johnny"));
    }

    #[test]
    fn test_write_name_type() {
        let source =
            "0 HEAD\n1 GEDC\n2 VERS 5.5\n0 @I1@ INDI\n1 NAME John /Doe/\n2 TYPE MARRIED\n0 TRLR";
        let data = GedcomBuilder::new().build_from_str(source).unwrap();

        let writer = GedcomWriter::new();
        let output = writer.write_to_string(&data).unwrap();

        assert!(output.contains("2 TYPE MARRIED"));
    }

    #[test]
    fn test_write_family_link_pedigree_and_adopted_by() {
        let source = "0 HEAD\n1 GEDC\n2 VERS 5.5\n0 @I1@ INDI\n1 FAMC @F1@\n2 PEDI adopted\n\
                       2 ADOP HUSB\n0 TRLR";
        let data = GedcomBuilder::new().build_from_str(source).unwrap();

        let writer = GedcomWriter::new();
        let output = writer.write_to_string(&data).unwrap();

        assert!(output.contains("1 FAMC @F1@"));
        assert!(output.contains("2 PEDI adopted"));
        assert!(output.contains("2 ADOP HUSB"));
    }

    #[test]
    fn test_write_adoption_event_family_link() {
        let source = "0 HEAD\n1 GEDC\n2 VERS 5.5\n0 @I1@ INDI\n1 ADOP\n2 FAMC @F2@\n\
                       3 ADOP WIFE\n0 TRLR";
        let data = GedcomBuilder::new().build_from_str(source).unwrap();

        let writer = GedcomWriter::new();
        let output = writer.write_to_string(&data).unwrap();

        assert!(output.contains("1 ADOP"));
        assert!(output.contains("2 FAMC @F2@"));
        assert!(output.contains("3 ADOP WIFE"));
    }

    #[test]
    fn test_write_event_association() {
        let source = "0 HEAD\n1 GEDC\n2 VERS 5.5\n0 @I1@ INDI\n1 BAPM\n2 ASSO @I2@\n\
                       3 RELA Godmother\n0 TRLR";
        let data = GedcomBuilder::new().build_from_str(source).unwrap();

        let writer = GedcomWriter::new();
        let output = writer.write_to_string(&data).unwrap();

        assert!(output.contains("1 BAPM"));
        assert!(output.contains("2 ASSO @I2@"));
        assert!(output.contains("3 RELA Godmother"));
    }

    #[test]
    fn test_event_multimedia_individual() {
        let source = "\
            0 HEAD\n\
            1 GEDC\n\
            2 VERS 5.5\n\
            0 @I1@ INDI\n\
            1 BIRT\n\
            2 OBJE @M1@\n\
            0 TRLR";

        let data = GedcomBuilder::new().build_from_str(source).unwrap();
        let writer = GedcomWriter::new();
        let output = writer.write_to_string(&data).unwrap();

        assert!(output.contains("2 OBJE @M1@"));
    }

    #[test]
    fn test_event_multimedia_family() {
        let source = "\
            0 HEAD\n\
            1 GEDC\n\
            2 VERS 5.5\n\
            0 @F1@ FAM\n\
            1 MARR\n\
            2 OBJE @M2@\n\
            0 TRLR";

        let data = GedcomBuilder::new().build_from_str(source).unwrap();
        let writer = GedcomWriter::new();
        let output = writer.write_to_string(&data).unwrap();

        assert!(output.contains("2 OBJE @M2@"));
    }

    #[test]
    fn test_event_multimedia_inline() {
        let source = "\
            0 HEAD\n\
            1 GEDC\n\
            2 VERS 5.5\n\
            0 @I1@ INDI\n\
            1 BIRT\n\
            2 OBJE\n\
            3 FILE photo.jpg\n\
            4 FORM jpeg\n\
            3 TITL baby photo\n\
            0 TRLR";

        let data = GedcomBuilder::new().build_from_str(source).unwrap();
        let writer = GedcomWriter::new();
        let output = writer.write_to_string(&data).unwrap();

        assert!(output.contains("2 OBJE"));
        assert!(output.contains("3 FILE photo.jpg"));
        assert!(output.contains("4 FORM jpeg"));
        assert!(output.contains("3 TITL baby photo"));
    }

    #[test]
    fn test_attribute_detail() {
        let source = "\
            0 HEAD\n\
            1 GEDC\n\
            2 VERS 5.5\n\
            0 @I1@ INDI\n\
            1 OCCU Baker\n\
            2 TYPE Trade\n\
            2 DATE 1880\n\
            2 PLAC London, England\n\
            2 ADDR 1 Main St\n\
            2 RESN privacy\n\
            2 AGE 40y\n\
            2 CAUS Guild record\n\
            2 AGNC Bakers Guild\n\
            2 SOUR @S1@\n\
            2 NOTE Apprenticed at 14\n\
            2 OBJE @M1@\n\
            0 TRLR";

        let data = GedcomBuilder::new().build_from_str(source).unwrap();
        let writer = GedcomWriter::new();
        let output = writer.write_to_string(&data).unwrap();

        assert!(output.contains("1 OCCU Baker"));
        assert!(output.contains("2 TYPE Trade"));
        assert!(output.contains("2 DATE 1880"));
        assert!(output.contains("2 PLAC London, England"));
        assert!(output.contains("2 ADDR 1 Main St"));
        assert!(output.contains("2 RESN privacy"));
        assert!(output.contains("2 AGE 40y"));
        assert!(output.contains("2 CAUS Guild record"));
        assert!(output.contains("2 AGNC Bakers Guild"));
        assert!(output.contains("2 SOUR @S1@"));
        assert!(output.contains("2 NOTE Apprenticed at 14"));
        assert!(output.contains("2 OBJE @M1@"));
    }

    #[test]
    fn test_write_long_utf8_text_splits_on_char_boundary() {
        // `1 NOTE `, the delimiter and the line feed leave 247 bytes, which
        // end inside `é`.
        let note = format!("{}é continued", "a".repeat(246));
        let source = format!("0 HEAD\n1 GEDC\n2 VERS 5.5\n0 @I1@ INDI\n1 NOTE {note}\n0 TRLR");
        let data = GedcomBuilder::new().build_from_str(&source).unwrap();

        let writer = GedcomWriter::new();
        let output = writer.write_to_string(&data).unwrap();

        assert!(output.contains("1 NOTE"));
        assert!(output.contains("2 CONC é continued"));
    }

    #[test]
    fn test_writer_config() {
        let writer = GedcomWriter::new()
            .line_ending(LineEnding::CrLf)
            .max_line_length(100)
            .gedcom_version(GedcomVersion::V5_5_1);

        let config = writer.config();
        assert_eq!(config.line_ending, LineEnding::CrLf);
        assert_eq!(config.max_line_length, 100);
        assert_eq!(config.version, Some(GedcomVersion::V5_5_1));
    }
}
