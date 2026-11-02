//! The file an adapter reads.

use sha2::{Digest, Sha256};

use crate::manifest::FileKind;

pub struct FileInput {
    pub name: String,
    pub bytes: Vec<u8>,
    /// Lower-case hex SHA-256 of `bytes`.
    pub sha256: String,
    pub kind: FileKind,
}

impl FileInput {
    pub fn new(name: impl Into<String>, bytes: Vec<u8>) -> Self {
        let name = name.into();
        let sha256 = hex::encode(Sha256::digest(&bytes));
        let kind = sniff_kind(&name, &bytes);
        FileInput {
            name,
            bytes,
            sha256,
            kind,
        }
    }
}

/// File kind from magic bytes first, then the extension.
pub fn sniff_kind(name: &str, bytes: &[u8]) -> FileKind {
    let head = &bytes[..bytes.len().min(512)];
    let text = String::from_utf8_lossy(head);
    let trimmed = text.trim_start_matches('\u{feff}').trim_start();
    if head.starts_with(b"%PDF") {
        return FileKind::Pdf;
    }
    if head.starts_with(b"PK\x03\x04") {
        return FileKind::Xlsx;
    }
    if trimmed.starts_with("OFXHEADER") || trimmed.contains("<OFX>") || trimmed.contains("<?OFX") {
        return FileKind::Ofx;
    }
    if trimmed.starts_with("<?xml") || trimmed.starts_with('<') {
        return FileKind::Xml;
    }
    let ext = name.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    match ext.as_str() {
        "csv" | "tsv" => FileKind::Csv,
        "xlsx" | "xlsm" => FileKind::Xlsx,
        "pdf" => FileKind::Pdf,
        "ofx" | "qfx" => FileKind::Ofx,
        "xml" => FileKind::Xml,
        _ => FileKind::Text,
    }
}

// ---------------------------------------------------------------- tables and text

/// How to read a CSV or XLSX file as a table.
#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Deserialize, serde::Serialize)]
#[serde(default, deny_unknown_fields)]
pub struct TableOptions {
    /// CSV field delimiter (one character). Guessed from the header line when absent.
    pub delimiter: Option<String>,
    /// CSV text encoding label, e.g. `windows-1254`. UTF-8 (with or without BOM) by default.
    pub encoding: Option<String>,
    /// Rows to skip before the header row.
    pub skip_rows: usize,
    /// XLSX sheet name; the first sheet by default.
    pub sheet: Option<String>,
}

/// A header plus data rows, each with its 1-based row number in the source.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Table {
    pub header: Vec<String>,
    pub rows: Vec<(usize, Vec<String>)>,
}

impl Table {
    pub fn column(&self, name: &str) -> Option<usize> {
        self.header.iter().position(|h| h.trim() == name.trim())
    }
}

impl FileInput {
    /// Decode the file as text. Strips a UTF-8 BOM; `encoding` is a WHATWG label.
    pub fn text(&self, encoding: Option<&str>) -> Result<String, String> {
        let enc = match encoding {
            None => encoding_rs::UTF_8,
            Some(l) => encoding_rs::Encoding::for_label(l.as_bytes())
                .ok_or_else(|| format!("unknown encoding {l:?}"))?,
        };
        let (text, _, had_errors) = enc.decode(&self.bytes);
        if had_errors && encoding.is_none() {
            return Err(
                "the file is not valid UTF-8; set its encoding (for example windows-1254)".into(),
            );
        }
        Ok(text.trim_start_matches('\u{feff}').to_string())
    }

    /// A likely encoding for a CSV that is not UTF-8: Turkish Windows when the
    /// bytes include its letters (İ ı Ş ş Ğ ğ), otherwise Western Windows.
    /// `None` when the file is valid UTF-8.
    pub fn guess_encoding(&self) -> Option<&'static str> {
        if std::str::from_utf8(&self.bytes).is_ok() {
            return None;
        }
        let turkish = self
            .bytes
            .iter()
            .any(|b| matches!(b, 0xDD | 0xFD | 0xDE | 0xFE | 0xD0 | 0xF0));
        Some(if turkish {
            "windows-1254"
        } else {
            "windows-1252"
        })
    }

    /// The delimiter a CSV would be read with when none is set.
    pub fn guess_delimiter(&self, opt: &TableOptions) -> Option<String> {
        if self.kind != FileKind::Csv {
            return None;
        }
        let text = self
            .text(opt.encoding.as_deref().or(self.guess_encoding()))
            .ok()?;
        let line = text
            .lines()
            .filter(|l| !l.trim().is_empty())
            .nth(opt.skip_rows)
            .unwrap_or("");
        Some((guess_delimiter(line) as char).to_string())
    }

    /// Sheet names of an XLSX workbook; empty for other files.
    pub fn sheet_names(&self) -> Vec<String> {
        use calamine::{open_workbook_from_rs, Reader, Xlsx};
        if self.kind != FileKind::Xlsx {
            return Vec::new();
        }
        open_workbook_from_rs::<Xlsx<_>, _>(std::io::Cursor::new(self.bytes.as_slice()))
            .map(|wb| wb.sheet_names())
            .unwrap_or_default()
    }

    /// Read the file as a table (CSV or XLSX, by its kind).
    pub fn table(&self, opt: &TableOptions) -> Result<Table, String> {
        match self.kind {
            FileKind::Xlsx => self.xlsx_table(opt),
            _ => self.csv_table(opt),
        }
    }

    fn csv_table(&self, opt: &TableOptions) -> Result<Table, String> {
        let text = self.text(opt.encoding.as_deref())?;
        let delimiter = match opt.delimiter.as_deref() {
            Some(d) if d.len() == 1 => d.as_bytes()[0],
            Some("\\t") | Some("tab") => b'\t',
            Some(d) => return Err(format!("delimiter must be one character, got {d:?}")),
            None => guess_delimiter(text.lines().nth(opt.skip_rows).unwrap_or("")),
        };
        let mut rdr = csv::ReaderBuilder::new()
            .delimiter(delimiter)
            .has_headers(false)
            .flexible(true)
            .from_reader(text.as_bytes());
        let mut header = None;
        let mut rows = Vec::new();
        for (i, rec) in rdr.records().enumerate() {
            let rec = rec.map_err(|e| format!("CSV error: {e}"))?;
            // Line from the byte offset, so blank lines and multi-line fields count correctly.
            let line = rec
                .position()
                .map(|p| {
                    let bytes = text.as_bytes();
                    let mut at = p.byte() as usize;
                    // The reader reports skipped blank lines as part of the next record.
                    while at < bytes.len() && (bytes[at] == b'\n' || bytes[at] == b'\r') {
                        at += 1;
                    }
                    bytes[..at].iter().filter(|b| **b == b'\n').count() + 1
                })
                .unwrap_or(i + 1);
            if i < opt.skip_rows {
                continue;
            }
            let cells: Vec<String> = rec.iter().map(|c| c.trim().to_string()).collect();
            if header.is_none() {
                header = Some(cells);
            } else if cells.iter().any(|c| !c.is_empty()) {
                rows.push((line, cells));
            }
        }
        Ok(Table {
            header: header.ok_or("the file has no header row")?,
            rows,
        })
    }

    fn xlsx_table(&self, opt: &TableOptions) -> Result<Table, String> {
        use calamine::{open_workbook_from_rs, Data, Reader, Xlsx};
        let mut wb: Xlsx<_> = open_workbook_from_rs(std::io::Cursor::new(self.bytes.as_slice()))
            .map_err(|e| format!("cannot read XLSX: {e}"))?;
        let names = wb.sheet_names();
        let sheet = match &opt.sheet {
            Some(s) if names.contains(s) => s.clone(),
            Some(s) => {
                return Err(format!(
                    "sheet {s:?} not found; sheets: {}",
                    names.join(", ")
                ))
            }
            None => names.first().cloned().ok_or("the workbook has no sheets")?,
        };
        let range = wb
            .worksheet_range(&sheet)
            .map_err(|e| format!("cannot read sheet {sheet:?}: {e}"))?;
        let first_row = range.start().map(|(r, _)| r as usize).unwrap_or(0);
        let width = range.width();
        let mut header = None;
        let mut rows = Vec::new();
        for (i, row) in range.rows().enumerate() {
            if i < opt.skip_rows {
                continue;
            }
            let mut cells: Vec<String> = row
                .iter()
                .map(|c| match c {
                    Data::String(s) | Data::DateTimeIso(s) | Data::DurationIso(s) => {
                        s.trim().to_string()
                    }
                    Data::Int(n) => n.to_string(),
                    Data::Float(f) if f.fract() == 0.0 && f.abs() < 1e15 => {
                        format!("{}", *f as i64)
                    }
                    Data::Float(f) => f.to_string(),
                    Data::Bool(b) => b.to_string(),
                    Data::DateTime(d) => d.to_string(),
                    Data::Error(e) => format!("#{e:?}"),
                    Data::Empty => String::new(),
                })
                .collect();
            cells.resize(width, String::new());
            let line = first_row + i + 1;
            if header.is_none() {
                header = Some(cells);
            } else if cells.iter().any(|c| !c.is_empty()) {
                rows.push((line, cells));
            }
        }
        Ok(Table {
            header: header.ok_or("the sheet has no header row")?,
            rows,
        })
    }

    /// Text of each PDF page. Fails clearly for PDFs without a text layer.
    pub fn pdf_pages(&self) -> Result<Vec<String>, String> {
        let pages = pdf_extract::extract_text_from_mem_by_pages(&self.bytes)
            .map_err(|e| format!("cannot read PDF: {e}"))?;
        if pages.iter().all(|p| p.trim().is_empty()) {
            return Err(
                "this PDF has no text layer (scanned?); only PDFs with text can be imported".into(),
            );
        }
        Ok(pages)
    }
}

fn guess_delimiter(line: &str) -> u8 {
    (*b";,\t|")
        .into_iter()
        .max_by_key(|d| line.bytes().filter(|b| b == d).count())
        .filter(|d| line.as_bytes().contains(d))
        .unwrap_or(b',')
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fixture(name: &str) -> FileInput {
        let p = format!("{}/tests/fixtures/input/{name}", env!("CARGO_MANIFEST_DIR"));
        FileInput::new(name, std::fs::read(p).unwrap())
    }

    #[test]
    fn guesses_for_the_mapper() {
        let f = FileInput::new(
            "a.csv",
            b"Tarih;A\xe7\xfdklama;Tutar\n01.10.2026;\xddade;5\n".to_vec(),
        );
        assert_eq!(f.guess_encoding(), Some("windows-1254"));
        assert_eq!(
            f.guess_delimiter(&TableOptions::default()).as_deref(),
            Some(";")
        );
        assert_eq!(
            FileInput::new("b.csv", b"a,b\n".to_vec()).guess_encoding(),
            None
        );
        assert_eq!(fixture("two_sheets.xlsx").sheet_names().len(), 2);
    }

    #[test]
    fn utf8_bom_csv_with_guessed_delimiter() {
        let f = FileInput::new(
            "a.csv",
            "\u{feff}Tarih;Tutar\n01.10.2026;1.000,00\n\n02.10.2026;5,00\n"
                .as_bytes()
                .to_vec(),
        );
        let t = f.table(&TableOptions::default()).unwrap();
        assert_eq!(t.header, vec!["Tarih", "Tutar"]);
        assert_eq!(
            t.rows,
            vec![
                (2, vec!["01.10.2026".to_string(), "1.000,00".into()]),
                (4, vec!["02.10.2026".into(), "5,00".into()])
            ]
        );
    }

    #[test]
    fn windows_1254_csv_with_skipped_rows() {
        let src = "Banka Ekstresi\nHesap: 123\nTarih;Açıklama\n01.10.2026;Hesap İşletim Ücreti\n";
        let (bytes, _, _) = encoding_rs::WINDOWS_1254.encode(src);
        let f = FileInput::new("a.csv", bytes.into_owned());
        assert!(
            f.table(&TableOptions::default()).is_err(),
            "not UTF-8 without an encoding"
        );
        let opt = TableOptions {
            encoding: Some("windows-1254".into()),
            skip_rows: 2,
            delimiter: Some(";".into()),
            ..Default::default()
        };
        let t = f.table(&opt).unwrap();
        assert_eq!(t.header, vec!["Tarih", "Açıklama"]);
        assert_eq!(
            t.rows[0],
            (
                4,
                vec!["01.10.2026".to_string(), "Hesap İşletim Ücreti".into()]
            ),
            "row 4 of the file"
        );
    }

    #[test]
    fn xlsx_sheet_by_name() {
        let f = fixture("two_sheets.xlsx");
        assert_eq!(f.kind, FileKind::Xlsx);
        let t = f
            .table(&TableOptions {
                sheet: Some("Hesap Hareketleri".into()),
                skip_rows: 1,
                ..Default::default()
            })
            .unwrap();
        assert_eq!(t.header, vec!["Tarih", "Açıklama", "Tutar"]);
        assert_eq!(
            t.rows[0],
            (
                3,
                vec!["01.10.2026".to_string(), "Havale".into(), "1.000,00".into()]
            )
        );
        assert_eq!(t.rows[1].1[2], "12.5", "numeric cells as displayed text");
        let first = f.table(&TableOptions::default()).unwrap();
        assert_eq!(
            first.header,
            vec!["Toplam", "1.234,56"],
            "first sheet by default"
        );
        assert!(f
            .table(&TableOptions {
                sheet: Some("Nope".into()),
                ..Default::default()
            })
            .unwrap_err()
            .contains("Hesap Hareketleri"));
    }

    #[test]
    fn pdf_text_and_scanned_pdf() {
        let pages = fixture("text_two_pages.pdf").pdf_pages().unwrap();
        assert_eq!(pages.len(), 2);
        assert!(pages[0].contains("Statement page one"), "{pages:?}");
        assert!(pages[1].contains("Closing balance 100.00"));
        let err = fixture("no_text.pdf").pdf_pages().unwrap_err();
        assert!(err.contains("no text layer"), "{err}");
    }
}
