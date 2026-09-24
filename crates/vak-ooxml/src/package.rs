//! L0: the bounded OPC package reader, inspection and raw-copy writer.

use std::collections::{BTreeMap, HashMap};
use std::io::{Read, Seek, SeekFrom, Write};

use serde::Serialize;

use crate::xml::{self, XmlEvent};
use crate::{Error, Limits};

const CFB_MAGIC: [u8; 8] = [0xD0, 0xCF, 0x11, 0xE0, 0xA1, 0xB1, 0x1A, 0xE1];
const CONTENT_TYPES: &str = "[Content_Types].xml";
const PACKAGE_RELS: &str = "_rels/.rels";

const REL_OFFICE_DOCUMENT: &str =
    "http://schemas.openxmlformats.org/officeDocument/2006/relationships/officeDocument";
const REL_OFFICE_DOCUMENT_STRICT: &str =
    "http://purl.oclc.org/ooxml/officeDocument/relationships/officeDocument";
const REL_VISIO_DOCUMENT: &str = "http://schemas.microsoft.com/visio/2010/relationships/document";
const REL_SIGNATURE_ORIGIN: &str =
    "http://schemas.openxmlformats.org/package/2006/relationships/digital-signature/origin";

const CT_VBA_PROJECT: &str = "application/vnd.ms-office.vbaproject";
const CT_ACTIVEX: &str = "application/vnd.ms-office.activex+xml";
const CT_SIGNATURE: &str =
    "application/vnd.openxmlformats-package.digital-signature-xmlsignature+xml";
const CT_XLM_MACROSHEET: &str = "application/vnd.ms-excel.macrosheet+xml";
const CT_XLM_INTL_MACROSHEET: &str = "application/vnd.ms-excel.intlmacrosheet+xml";
const CT_CUSTOM_XML_PROPS: &str =
    "application/vnd.openxmlformats-officedocument.customxmlproperties+xml";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Vocabulary {
    Word,
    Excel,
    PowerPoint,
    Visio,
}

impl Vocabulary {
    pub fn label(self) -> &'static str {
        match self {
            Vocabulary::Word => "Word document",
            Vocabulary::Excel => "Excel workbook",
            Vocabulary::PowerPoint => "PowerPoint presentation",
            Vocabulary::Visio => "Visio drawing",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum FormatKind {
    Document,
    Template,
    Slideshow,
    AddIn,
    Stencil,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
pub struct Format {
    pub vocabulary: Vocabulary,
    pub kind: FormatKind,
    pub macro_enabled: bool,
}

impl Format {
    /// Classifies a main part by its content type. Detection is by content
    /// type, never by the file extension (a renamed package lies about it).
    pub fn from_main_content_type(content_type: &str) -> Result<Self, Error> {
        use FormatKind::*;
        use Vocabulary::*;
        let lower = content_type.to_ascii_lowercase();
        let (vocabulary, kind, macro_enabled) = match lower.as_str() {
            "application/vnd.openxmlformats-officedocument.wordprocessingml.document.main+xml" => {
                (Word, Document, false)
            }
            "application/vnd.ms-word.document.macroenabled.main+xml" => (Word, Document, true),
            "application/vnd.openxmlformats-officedocument.wordprocessingml.template.main+xml" => {
                (Word, Template, false)
            }
            "application/vnd.ms-word.template.macroenabledtemplate.main+xml" => {
                (Word, Template, true)
            }
            "application/vnd.openxmlformats-officedocument.spreadsheetml.sheet.main+xml" => {
                (Excel, Document, false)
            }
            "application/vnd.ms-excel.sheet.macroenabled.main+xml" => (Excel, Document, true),
            "application/vnd.openxmlformats-officedocument.spreadsheetml.template.main+xml" => {
                (Excel, Template, false)
            }
            "application/vnd.ms-excel.template.macroenabled.main+xml" => (Excel, Template, true),
            "application/vnd.ms-excel.addin.macroenabled.main+xml" => (Excel, AddIn, true),
            "application/vnd.openxmlformats-officedocument.presentationml.presentation.main+xml" => {
                (PowerPoint, Document, false)
            }
            "application/vnd.ms-powerpoint.presentation.macroenabled.main+xml" => {
                (PowerPoint, Document, true)
            }
            "application/vnd.openxmlformats-officedocument.presentationml.template.main+xml" => {
                (PowerPoint, Template, false)
            }
            "application/vnd.ms-powerpoint.template.macroenabled.main+xml" => {
                (PowerPoint, Template, true)
            }
            "application/vnd.openxmlformats-officedocument.presentationml.slideshow.main+xml" => {
                (PowerPoint, Slideshow, false)
            }
            "application/vnd.ms-powerpoint.slideshow.macroenabled.main+xml" => {
                (PowerPoint, Slideshow, true)
            }
            "application/vnd.ms-powerpoint.addin.macroenabled.main+xml" => {
                (PowerPoint, AddIn, true)
            }
            "application/vnd.ms-visio.drawing.main+xml" => (Visio, Document, false),
            "application/vnd.ms-visio.drawing.macroenabled.main+xml" => (Visio, Document, true),
            "application/vnd.ms-visio.template.main+xml" => (Visio, Template, false),
            "application/vnd.ms-visio.template.macroenabled.main+xml" => (Visio, Template, true),
            "application/vnd.ms-visio.stencil.main+xml" => (Visio, Stencil, false),
            "application/vnd.ms-visio.stencil.macroenabled.main+xml" => (Visio, Stencil, true),
            _ => return Err(Error::UnsupportedFormat(content_type.to_string())),
        };
        Ok(Self {
            vocabulary,
            kind,
            macro_enabled,
        })
    }

    /// The one extension that names this format.
    pub fn extension(self) -> &'static str {
        use FormatKind::*;
        use Vocabulary::*;
        match (self.vocabulary, self.kind, self.macro_enabled) {
            (Word, Template, false) => "dotx",
            (Word, Template, true) => "dotm",
            (Word, _, false) => "docx",
            (Word, _, true) => "docm",
            (Excel, Template, false) => "xltx",
            (Excel, Template, true) => "xltm",
            (Excel, AddIn, _) => "xlam",
            (Excel, _, false) => "xlsx",
            (Excel, _, true) => "xlsm",
            (PowerPoint, Template, false) => "potx",
            (PowerPoint, Template, true) => "potm",
            (PowerPoint, Slideshow, false) => "ppsx",
            (PowerPoint, Slideshow, true) => "ppsm",
            (PowerPoint, AddIn, _) => "ppam",
            (PowerPoint, _, false) => "pptx",
            (PowerPoint, _, true) => "pptm",
            (Visio, Template, false) => "vstx",
            (Visio, Template, true) => "vstm",
            (Visio, Stencil, false) => "vssx",
            (Visio, Stencil, true) => "vssm",
            (Visio, _, false) => "vsdx",
            (Visio, _, true) => "vsdm",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum Conformance {
    Transitional,
    Strict,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Relationship {
    pub id: String,
    pub kind: String,
    pub target: String,
    pub external: bool,
}

impl Relationship {
    /// Last path segment of the relationship type URI (`image`, `comments`).
    pub fn short_kind(&self) -> &str {
        self.kind.rsplit('/').next().unwrap_or(&self.kind)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct ExternalRelationship {
    pub source: String,
    pub kind: String,
    pub target: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Entry {
    name: String,
    index: usize,
    size: u64,
}

#[derive(Debug, Default)]
struct ContentTypes {
    defaults: HashMap<String, String>,
    overrides: HashMap<String, String>,
}

/// What a read of the package observed, for flags and verification. Every
/// field is derived from the package itself; nothing here is executed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct Inspection {
    pub format: Format,
    pub conformance: Conformance,
    pub main_part: String,
    pub part_count: usize,
    pub vba_project: bool,
    pub xlm_macro_sheets: usize,
    pub activex_controls: usize,
    pub signed: bool,
    pub embedded_objects: usize,
    pub custom_xml_parts: usize,
    pub external_relationships: Vec<ExternalRelationship>,
    pub sensitivity_labels: Vec<String>,
}

impl Inspection {
    /// A remote template (`attachedTemplate` with an external target) is a
    /// known phishing and payload-delivery vector.
    pub fn remote_template(&self) -> bool {
        self.external_relationships
            .iter()
            .any(|relationship| relationship.kind == "attachedTemplate")
    }

    /// One-line human flags, empty when nothing is notable.
    pub fn flags(&self) -> Vec<String> {
        let mut flags = Vec::new();
        if self.conformance == Conformance::Strict {
            flags.push("Strict conformance".into());
        }
        if self.vba_project {
            flags.push("contains a VBA macro project (never executed)".into());
        } else if self.format.macro_enabled {
            flags.push("macro-enabled format without a VBA project".into());
        }
        if self.xlm_macro_sheets > 0 {
            flags.push(format!(
                "{} Excel 4.0 (XLM) macro sheet(s) (never executed)",
                self.xlm_macro_sheets
            ));
        }
        if self.activex_controls > 0 {
            flags.push(format!(
                "{} ActiveX control part(s) (never instantiated)",
                self.activex_controls
            ));
        }
        if self.signed {
            flags.push("digitally signed (signature not verified yet)".into());
        }
        if self.embedded_objects > 0 {
            flags.push(format!(
                "{} embedded object(s) (never activated)",
                self.embedded_objects
            ));
        }
        if self.remote_template() {
            flags.push("remote template reference (not followed; a known phishing vector)".into());
        }
        let other_external = self
            .external_relationships
            .iter()
            .filter(|relationship| relationship.kind != "attachedTemplate")
            .count();
        if other_external > 0 {
            flags.push(format!(
                "{other_external} external link(s) (recorded, never followed)"
            ));
        }
        if !self.sensitivity_labels.is_empty() {
            flags.push(format!(
                "sensitivity label: {}",
                self.sensitivity_labels.join(", ")
            ));
        }
        flags
    }
}

/// An opened package. Reading a part decompresses it through a bounded
/// reader and charges its actual size against the package total.
pub struct Package<R: Read + Seek> {
    archive: zip::ZipArchive<R>,
    limits: Limits,
    entries: Vec<Entry>,
    by_name: HashMap<String, usize>,
    content_types: ContentTypes,
    relationships: BTreeMap<String, Vec<Relationship>>,
    consumed: u64,
    format: Format,
    conformance: Conformance,
    main_part: String,
}

impl<R: Read + Seek> Package<R> {
    pub fn open(mut reader: R, limits: Limits) -> Result<Self, Error> {
        let mut magic = [0u8; 8];
        let read = read_prefix(&mut reader, &mut magic)?;
        if read == magic.len() && magic == CFB_MAGIC {
            return Err(Error::CompoundFile);
        }
        reader
            .seek(SeekFrom::Start(0))
            .map_err(|error| Error::Io(error.to_string()))?;
        let mut archive =
            zip::ZipArchive::new(reader).map_err(|error| Error::NotZip(error.to_string()))?;
        if archive.len() > limits.max_entries {
            return Err(Error::TooManyEntries(archive.len()));
        }
        let mut entries = Vec::with_capacity(archive.len());
        let mut by_name = HashMap::with_capacity(archive.len());
        let mut declared_total = 0u64;
        for index in 0..archive.len() {
            let file = archive
                .by_index_raw(index)
                .map_err(|error| Error::NotZip(error.to_string()))?;
            let name = std::str::from_utf8(file.name_raw())
                .map_err(|_| {
                    Error::InvalidPartName(String::from_utf8_lossy(file.name_raw()).into_owned())
                })?
                .to_string();
            validate_part_name(name.trim_end_matches('/'))?;
            if file.encrypted() {
                return Err(Error::EncryptedEntry(name));
            }
            if !matches!(
                file.compression(),
                zip::CompressionMethod::Stored | zip::CompressionMethod::Deflated
            ) {
                return Err(Error::UnsupportedCompression(name));
            }
            let size = file.size();
            if size > limits.max_part_bytes {
                return Err(Error::PartTooLarge(name));
            }
            let compressed = file.compressed_size().max(1);
            if size > limits.ratio_floor_bytes && size / compressed > limits.max_ratio {
                return Err(Error::CompressionRatio(name));
            }
            declared_total = declared_total.saturating_add(size);
            if declared_total > limits.max_total_bytes {
                return Err(Error::TotalTooLarge);
            }
            let key = name.trim_end_matches('/').to_ascii_lowercase();
            if by_name.insert(key, entries.len()).is_some() {
                return Err(Error::DuplicatePart(name));
            }
            drop(file);
            // Directory entries are not parts, but they stay indexed for
            // collision checks and are raw-copied on write.
            entries.push(Entry { name, index, size });
        }
        let mut package = Self {
            archive,
            limits,
            entries,
            by_name,
            content_types: ContentTypes::default(),
            relationships: BTreeMap::new(),
            consumed: 0,
            format: Format {
                vocabulary: Vocabulary::Word,
                kind: FormatKind::Document,
                macro_enabled: false,
            },
            conformance: Conformance::Transitional,
            main_part: String::new(),
        };
        package.content_types = package.read_content_types()?;
        let package_rels = package.relationships("")?.to_vec();
        let main = package_rels
            .iter()
            .find(|relationship| {
                !relationship.external
                    && matches!(
                        relationship.kind.as_str(),
                        REL_OFFICE_DOCUMENT | REL_OFFICE_DOCUMENT_STRICT | REL_VISIO_DOCUMENT
                    )
            })
            .ok_or(Error::NoMainPart)?;
        if main.kind == REL_OFFICE_DOCUMENT_STRICT {
            package.conformance = Conformance::Strict;
        }
        let main_part = main.target.clone();
        if !package.has_part(&main_part) {
            return Err(Error::MissingPart(main_part));
        }
        let content_type = package
            .content_type(&main_part)
            .ok_or_else(|| Error::UnsupportedFormat(String::new()))?;
        package.format = Format::from_main_content_type(&content_type)?;
        if package.format.vocabulary == Vocabulary::Visio
            && package.conformance == Conformance::Strict
        {
            return Err(Error::UnsupportedFormat(content_type));
        }
        package.main_part = main_part;
        Ok(package)
    }

    pub fn format(&self) -> Format {
        self.format
    }

    pub fn conformance(&self) -> Conformance {
        self.conformance
    }

    pub fn limits(&self) -> &Limits {
        &self.limits
    }

    /// Part name of the main document part, without a leading slash.
    pub fn main_part(&self) -> &str {
        &self.main_part
    }

    /// Part names in archive order, excluding directory entries.
    pub fn part_names(&self) -> impl Iterator<Item = &str> {
        self.entries
            .iter()
            .filter(|entry| !entry.name.ends_with('/'))
            .map(|entry| entry.name.as_str())
    }

    pub fn has_part(&self, name: &str) -> bool {
        self.entry(name).is_some()
    }

    fn entry(&self, name: &str) -> Option<&Entry> {
        self.by_name
            .get(&name.trim_start_matches('/').to_ascii_lowercase())
            .map(|index| &self.entries[*index])
            .filter(|entry| !entry.name.ends_with('/'))
    }

    /// Content type of a part: its `Override`, else the `Default` for its
    /// extension.
    pub fn content_type(&self, name: &str) -> Option<String> {
        let key = name.trim_start_matches('/').to_ascii_lowercase();
        if let Some(value) = self.content_types.overrides.get(&key) {
            return Some(value.clone());
        }
        let extension = key.rsplit_once('.').map(|(_, extension)| extension)?;
        self.content_types.defaults.get(extension).cloned()
    }

    /// Decompresses one part through a bounded reader.
    pub fn read_part(&mut self, name: &str) -> Result<Vec<u8>, Error> {
        let entry = self
            .entry(name)
            .cloned()
            .ok_or_else(|| Error::MissingPart(name.to_string()))?;
        let file = self
            .archive
            .by_index(entry.index)
            .map_err(|error| Error::Io(error.to_string()))?;
        let mut bytes = Vec::with_capacity(entry.size.min(self.limits.max_part_bytes) as usize);
        file.take(self.limits.max_part_bytes + 1)
            .read_to_end(&mut bytes)
            .map_err(|error| Error::Io(format!("{}: {error}", entry.name)))?;
        if bytes.len() as u64 > self.limits.max_part_bytes {
            return Err(Error::PartTooLarge(entry.name));
        }
        self.consumed = self.consumed.saturating_add(bytes.len() as u64);
        if self.consumed > self.limits.max_total_bytes {
            return Err(Error::TotalTooLarge);
        }
        Ok(bytes)
    }

    /// Relationships whose source is `source` (`""` for the package).
    /// A part with no relationships part has none.
    pub fn relationships(&mut self, source: &str) -> Result<&[Relationship], Error> {
        let source = source.trim_start_matches('/').to_string();
        if !self.relationships.contains_key(&source) {
            let rels_part = rels_part_name(&source);
            let parsed = if self.has_part(&rels_part) {
                if self.relationships.len() >= self.limits.max_relationship_parts {
                    return Err(Error::TooManyEntries(self.relationships.len()));
                }
                let bytes = self.read_part(&rels_part)?;
                parse_relationships(&bytes, &rels_part, &source, &self.limits)?
            } else if source.is_empty() {
                return Err(Error::MissingPart(PACKAGE_RELS.into()));
            } else {
                Vec::new()
            };
            self.relationships.insert(source.clone(), parsed);
        }
        Ok(self
            .relationships
            .get(&source)
            .map(Vec::as_slice)
            .unwrap_or(&[]))
    }

    /// Internal target of the first relationship of `source` whose type
    /// ends with `/<short_kind>`.
    pub fn related_part(
        &mut self,
        source: &str,
        short_kind: &str,
    ) -> Result<Option<String>, Error> {
        Ok(self
            .relationships(source)?
            .iter()
            .find(|relationship| !relationship.external && relationship.short_kind() == short_kind)
            .map(|relationship| relationship.target.clone()))
    }

    /// Internal target of relationship `id` of `source`.
    pub fn part_by_relationship_id(
        &mut self,
        source: &str,
        id: &str,
    ) -> Result<Option<String>, Error> {
        Ok(self
            .relationships(source)?
            .iter()
            .find(|relationship| !relationship.external && relationship.id == id)
            .map(|relationship| relationship.target.clone()))
    }

    /// Parses the main part's root and checks it names the vocabulary the
    /// content type claims (`document`, `workbook`, `presentation`,
    /// `VisioDocument`).
    pub fn check_main_root(&mut self) -> Result<(), Error> {
        let expected = match self.format.vocabulary {
            Vocabulary::Word => "document",
            Vocabulary::Excel => "workbook",
            Vocabulary::PowerPoint => "presentation",
            Vocabulary::Visio => "VisioDocument",
        };
        let main = self.main_part.clone();
        let bytes = self.read_part(&main)?;
        let root = xml::root(&bytes, &main, &self.limits)?;
        if root.local() != expected {
            return Err(Error::Xml {
                part: main,
                message: format!("root is {}, expected {expected}", root.local()),
            });
        }
        Ok(())
    }

    /// Everything a reader should flag. Reads every relationship part, the
    /// custom properties and nothing else; executes nothing.
    pub fn inspect(&mut self) -> Result<Inspection, Error> {
        let names: Vec<String> = self.part_names().map(str::to_string).collect();
        let mut inspection = Inspection {
            format: self.format,
            conformance: self.conformance,
            main_part: self.main_part.clone(),
            part_count: names.len(),
            vba_project: false,
            xlm_macro_sheets: 0,
            activex_controls: 0,
            signed: false,
            embedded_objects: 0,
            custom_xml_parts: 0,
            external_relationships: Vec::new(),
            sensitivity_labels: Vec::new(),
        };
        for name in &names {
            let content_type = self
                .content_type(name)
                .unwrap_or_default()
                .to_ascii_lowercase();
            let lower = name.to_ascii_lowercase();
            match content_type.as_str() {
                CT_VBA_PROJECT => inspection.vba_project = true,
                CT_ACTIVEX => inspection.activex_controls += 1,
                CT_SIGNATURE => inspection.signed = true,
                CT_XLM_MACROSHEET | CT_XLM_INTL_MACROSHEET => inspection.xlm_macro_sheets += 1,
                CT_CUSTOM_XML_PROPS => inspection.custom_xml_parts += 1,
                _ => {}
            }
            if lower.ends_with("vbaproject.bin") {
                inspection.vba_project = true;
            }
            if lower.contains("/embeddings/") {
                inspection.embedded_objects += 1;
            }
            if let Some(source) = rels_source(name) {
                for relationship in self.relationships(&source)?.to_vec() {
                    if relationship.external {
                        inspection
                            .external_relationships
                            .push(ExternalRelationship {
                                source: if source.is_empty() {
                                    "package".into()
                                } else {
                                    source.clone()
                                },
                                kind: relationship.short_kind().to_string(),
                                target: relationship.target.clone(),
                            });
                    }
                }
            }
        }
        if self
            .relationships("")?
            .iter()
            .any(|relationship| relationship.kind == REL_SIGNATURE_ORIGIN)
        {
            inspection.signed = true;
        }
        if let Some(custom) = self
            .related_part("", "custom-properties")?
            .filter(|part| self.has_part(part))
        {
            let bytes = self.read_part(&custom)?;
            inspection.sensitivity_labels = sensitivity_labels(&bytes, &custom, &self.limits)?;
        }
        Ok(inspection)
    }

    /// Writes the package to `out`, replacing the parts named in `edits`
    /// and copying every other entry's compressed bytes unchanged (O1).
    /// Entries keep their archive order; edits that name no existing part
    /// are appended in name order. Written entries carry a fixed timestamp
    /// so the same edits always produce the same bytes.
    pub fn rewrite<W: Write + Seek>(
        &mut self,
        out: W,
        edits: &BTreeMap<String, Vec<u8>>,
    ) -> Result<W, Error> {
        let mut pending: BTreeMap<String, &Vec<u8>> = BTreeMap::new();
        for (name, bytes) in edits {
            let name = name.trim_start_matches('/');
            validate_part_name(name)?;
            if pending.insert(name.to_ascii_lowercase(), bytes).is_some() {
                return Err(Error::DuplicatePart(name.to_string()));
            }
        }
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Deflated)
            .last_modified_time(zip::DateTime::default());
        let mut writer = zip::ZipWriter::new(out);
        let io = |error: zip::result::ZipError| Error::Io(error.to_string());
        for entry in self.entries.clone() {
            match pending.remove(&entry.name.to_ascii_lowercase()) {
                Some(bytes) => {
                    writer
                        .start_file(entry.name.as_str(), options)
                        .map_err(io)?;
                    writer
                        .write_all(bytes)
                        .map_err(|error| Error::Io(error.to_string()))?;
                }
                None => {
                    let file = self.archive.by_index_raw(entry.index).map_err(io)?;
                    writer.raw_copy_file(file).map_err(io)?;
                }
            }
        }
        let added: Vec<(String, &Vec<u8>)> = edits
            .iter()
            .filter(|(name, _)| {
                pending.contains_key(&name.trim_start_matches('/').to_ascii_lowercase())
            })
            .map(|(name, bytes)| (name.trim_start_matches('/').to_string(), bytes))
            .collect();
        for (name, bytes) in added {
            writer.start_file(name, options).map_err(io)?;
            writer
                .write_all(bytes)
                .map_err(|error| Error::Io(error.to_string()))?;
        }
        writer.finish().map_err(io)
    }

    fn read_content_types(&mut self) -> Result<ContentTypes, Error> {
        let bytes = self.read_part(CONTENT_TYPES).map_err(|error| match error {
            Error::MissingPart(_) => Error::MissingPart(CONTENT_TYPES.into()),
            other => other,
        })?;
        let mut types = ContentTypes::default();
        let mut saw_types_root = false;
        xml::walk(&bytes, CONTENT_TYPES, &self.limits, |event| {
            if let XmlEvent::Open(element) = event {
                match element.local() {
                    "Types" => saw_types_root = true,
                    "Default" => {
                        if let (Some(extension), Some(content_type)) =
                            (element.attr("Extension"), element.attr("ContentType"))
                        {
                            types
                                .defaults
                                .insert(extension.to_ascii_lowercase(), content_type.to_string());
                        }
                    }
                    "Override" => {
                        if let (Some(part), Some(content_type)) =
                            (element.attr("PartName"), element.attr("ContentType"))
                        {
                            types.overrides.insert(
                                part.trim_start_matches('/').to_ascii_lowercase(),
                                content_type.to_string(),
                            );
                        }
                    }
                    _ => {}
                }
            }
            Ok(())
        })?;
        if !saw_types_root {
            return Err(Error::Xml {
                part: CONTENT_TYPES.into(),
                message: "root is not Types".into(),
            });
        }
        Ok(types)
    }
}

fn read_prefix(reader: &mut impl Read, buffer: &mut [u8]) -> Result<usize, Error> {
    let mut filled = 0;
    while filled < buffer.len() {
        match reader.read(&mut buffer[filled..]) {
            Ok(0) => break,
            Ok(count) => filled += count,
            Err(error) => return Err(Error::Io(error.to_string())),
        }
    }
    Ok(filled)
}

/// OPC part-name rules plus the refusals O2 names: no traversal, no
/// absolute or drive paths, no backslashes, no empty segments, no control
/// characters.
pub fn validate_part_name(name: &str) -> Result<(), Error> {
    let invalid = || Error::InvalidPartName(name.to_string());
    if name.is_empty() || name.starts_with('/') || name.contains('\\') {
        return Err(invalid());
    }
    if name.chars().any(|character| character.is_control()) {
        return Err(invalid());
    }
    if name.len() > 1024 {
        return Err(invalid());
    }
    for segment in name.split('/') {
        if segment.is_empty() || segment == "." || segment == ".." || segment.ends_with('.') {
            return Err(invalid());
        }
        if segment.contains(':') {
            return Err(invalid());
        }
    }
    Ok(())
}

/// `word/document.xml` → `word/_rels/document.xml.rels`; `""` → `_rels/.rels`.
fn rels_part_name(source: &str) -> String {
    match source.rsplit_once('/') {
        Some((directory, file)) => format!("{directory}/_rels/{file}.rels"),
        None if source.is_empty() => PACKAGE_RELS.into(),
        None => format!("_rels/{source}.rels"),
    }
}

/// Inverse of [`rels_part_name`]: the source part a relationships part
/// describes, or `None` when `name` is not a relationships part.
fn rels_source(name: &str) -> Option<String> {
    let lower = name.to_ascii_lowercase();
    if !lower.ends_with(".rels") {
        return None;
    }
    let (directory, file) = match name.rsplit_once('/') {
        Some((directory, file)) => (directory, file),
        None => return None,
    };
    let file = &file[..file.len() - ".rels".len()];
    let parent = if directory.eq_ignore_ascii_case("_rels") {
        ""
    } else {
        directory.strip_suffix("/_rels")?
    };
    Some(if parent.is_empty() {
        file.to_string()
    } else {
        format!("{parent}/{file}")
    })
}

fn parse_relationships(
    bytes: &[u8],
    part: &str,
    source: &str,
    limits: &Limits,
) -> Result<Vec<Relationship>, Error> {
    let mut relationships = Vec::new();
    let mut failure = None;
    xml::walk(bytes, part, limits, |event| {
        if let XmlEvent::Open(element) = event
            && element.local() == "Relationship"
        {
            let (Some(id), Some(kind), Some(target)) = (
                element.attr("Id"),
                element.attr("Type"),
                element.attr("Target"),
            ) else {
                return Ok(());
            };
            let external = element
                .attr("TargetMode")
                .is_some_and(|mode| mode.eq_ignore_ascii_case("External"));
            let target = if external {
                target.to_string()
            } else {
                match resolve_target(source, target) {
                    Ok(resolved) => resolved,
                    Err(error) => {
                        failure.get_or_insert(error);
                        return Ok(());
                    }
                }
            };
            relationships.push(Relationship {
                id: id.to_string(),
                kind: kind.to_string(),
                target,
                external,
            });
        }
        Ok(())
    })?;
    if let Some(error) = failure {
        return Err(error);
    }
    Ok(relationships)
}

/// Resolves an internal relationship target against its source part.
/// A target that climbs above the package root is refused.
pub fn resolve_target(source: &str, target: &str) -> Result<String, Error> {
    let target = target.split('#').next().unwrap_or("");
    let decoded = percent_decode(target).ok_or_else(|| Error::RelationshipEscape(target.into()))?;
    if decoded.contains('\\') || decoded.contains(':') {
        return Err(Error::RelationshipEscape(target.to_string()));
    }
    let mut segments: Vec<&str> = if decoded.starts_with('/') {
        Vec::new()
    } else {
        match source.rsplit_once('/') {
            Some((directory, _)) => directory.split('/').collect(),
            None => Vec::new(),
        }
    };
    for segment in decoded.split('/') {
        match segment {
            "" | "." => {}
            ".." => {
                if segments.pop().is_none() {
                    return Err(Error::RelationshipEscape(target.to_string()));
                }
            }
            other => segments.push(other),
        }
    }
    if segments.is_empty() {
        return Err(Error::RelationshipEscape(target.to_string()));
    }
    Ok(segments.join("/"))
}

fn percent_decode(value: &str) -> Option<String> {
    let bytes = value.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut index = 0;
    while index < bytes.len() {
        if bytes[index] == b'%' {
            let hex = value.get(index + 1..index + 3)?;
            out.push(u8::from_str_radix(hex, 16).ok()?);
            index += 3;
        } else {
            out.push(bytes[index]);
            index += 1;
        }
    }
    String::from_utf8(out).ok()
}

/// Microsoft Information Protection labels recorded as custom properties
/// (`MSIP_Label_<guid>_Name`). Labels are read to narrow egress only (O9).
fn sensitivity_labels(bytes: &[u8], part: &str, limits: &Limits) -> Result<Vec<String>, Error> {
    let mut labels = Vec::new();
    let mut current: Option<String> = None;
    let mut text = String::new();
    xml::walk(bytes, part, limits, |event| {
        match event {
            XmlEvent::Open(element) if element.local() == "property" => {
                current = element
                    .attr("name")
                    .filter(|name| name.starts_with("MSIP_Label_") && name.ends_with("_Name"))
                    .map(str::to_string);
                text.clear();
            }
            XmlEvent::Text(value) if current.is_some() => text.push_str(&value),
            XmlEvent::Close(name) if xml::local_name(&name) == "property" => {
                let label = current.take().map(|_| text.trim().to_string());
                labels.extend(label.filter(|label| !label.is_empty()));
            }
            _ => {}
        }
        Ok(())
    })?;
    labels.sort();
    labels.dedup();
    Ok(labels)
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::expect_used)]
mod tests {
    use super::*;

    #[test]
    fn part_names_refuse_traversal_and_odd_shapes() {
        for bad in [
            "../evil.xml",
            "word/../../evil.xml",
            "/abs.xml",
            "word\\document.xml",
            "word//document.xml",
            "C:/x.xml",
            "word/./document.xml",
            "word/document.xml.",
            "a\u{0}b",
        ] {
            assert!(validate_part_name(bad).is_err(), "{bad} should be refused");
        }
        validate_part_name("word/document.xml").unwrap();
        validate_part_name("[Content_Types].xml").unwrap();
    }

    #[test]
    fn targets_resolve_relative_to_source_and_never_escape() {
        assert_eq!(
            resolve_target("word/document.xml", "media/image1.png").unwrap(),
            "word/media/image1.png"
        );
        assert_eq!(
            resolve_target("ppt/slides/slide1.xml", "../slideLayouts/slideLayout1.xml").unwrap(),
            "ppt/slideLayouts/slideLayout1.xml"
        );
        assert_eq!(
            resolve_target("", "/word/document.xml").unwrap(),
            "word/document.xml"
        );
        assert_eq!(
            resolve_target("", "word/a%20b.xml").unwrap(),
            "word/a b.xml"
        );
        assert!(resolve_target("word/document.xml", "../../etc/passwd").is_err());
        assert!(resolve_target("", "..").is_err());
    }

    #[test]
    fn rels_names_round_trip() {
        for source in ["", "word/document.xml", "ppt/slides/slide1.xml"] {
            assert_eq!(rels_source(&rels_part_name(source)).unwrap(), source);
        }
        assert_eq!(rels_source("word/document.xml"), None);
    }

    #[test]
    fn every_extension_maps_back_from_its_format() {
        for extension in crate::EXTENSIONS {
            assert!(crate::is_openxml_path(&format!("x.{extension}")));
        }
        let format =
            Format::from_main_content_type("application/vnd.ms-excel.sheet.macroEnabled.main+xml")
                .unwrap();
        assert_eq!(format.extension(), "xlsm");
        assert!(
            Format::from_main_content_type(
                "application/vnd.ms-excel.sheet.binary.macroEnabled.main"
            )
            .is_err()
        );
    }
}
