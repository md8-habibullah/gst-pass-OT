//! Reading text tables: character set handling (BOMs, UTF-16, lossy UTF-8), delimiter detection
//! and a streaming CSV reader with a truncation check.

use super::limits::{
    cap_chars, unsupported, MAX_CSV_COLUMNS, MAX_ENTRIES, MSG_DAMAGED, MSG_NOT_TEXT, MSG_TOO_MANY,
    MSG_UNCLOSED_QUOTE, SAMPLE_CELL_CHARS, SAMPLE_ROWS,
};
use crate::storage::StorageError;
use zeroize::Zeroizing;

/// Decoded file text. Owned text (converted from UTF-16 or repaired) is wiped on drop.
pub enum Text<'a> {
    Borrowed(&'a str),
    Owned(Zeroizing<String>),
}

impl Text<'_> {
    pub fn as_str(&self) -> &str {
        match self {
            Text::Borrowed(s) => s,
            Text::Owned(s) => s.as_str(),
        }
    }
}

/// Turn file bytes into text: a UTF-8 byte order mark is dropped, UTF-16 (with a mark) is
/// converted, invalid UTF-8 is repaired with U+FFFD as long as it is rare, and anything that looks
/// binary (NUL bytes, many invalid sequences) is refused.
pub fn decode(bytes: &[u8]) -> Result<Text<'_>, StorageError> {
    if let Some(rest) = bytes.strip_prefix(&[0xFF, 0xFE]) {
        return Ok(Text::Owned(utf16(rest, true)));
    }
    if let Some(rest) = bytes.strip_prefix(&[0xFE, 0xFF]) {
        return Ok(Text::Owned(utf16(rest, false)));
    }
    let body = bytes.strip_prefix(&[0xEF, 0xBB, 0xBF]).unwrap_or(bytes);
    if body.iter().take(8192).any(|b| *b == 0) {
        return Err(unsupported(MSG_NOT_TEXT));
    }
    match std::str::from_utf8(body) {
        Ok(s) => Ok(Text::Borrowed(s)),
        Err(_) => {
            let fixed = Zeroizing::new(String::from_utf8_lossy(body).into_owned());
            let bad = fixed.matches('\u{FFFD}').count();
            // "Mostly fine": at most one repaired character in twenty.
            if bad * 20 > fixed.chars().count() {
                return Err(unsupported(MSG_NOT_TEXT));
            }
            Ok(Text::Owned(fixed))
        }
    }
}

fn utf16(bytes: &[u8], little: bool) -> Zeroizing<String> {
    let (pairs, _) = bytes.as_chunks::<2>();
    let units: Vec<u16> = pairs
        .iter()
        .map(|&p| {
            if little {
                u16::from_le_bytes(p)
            } else {
                u16::from_be_bytes(p)
            }
        })
        .collect();
    Zeroizing::new(String::from_utf16_lossy(&units))
}

/// Lower-case letters and digits only: `Login_URI` -> `loginuri`.
pub fn norm(header: &str) -> String {
    header
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .map(|c| c.to_ascii_lowercase())
        .collect()
}

fn reader(text: &str, delim: u8) -> csv::Reader<&[u8]> {
    csv::ReaderBuilder::new()
        .has_headers(false)
        .flexible(true)
        .delimiter(delim)
        .double_quote(true)
        .quoting(true)
        .comment(None)
        .buffer_capacity(64 * 1024)
        .from_reader(text.as_bytes())
}

/// Pick `,` `;` tab or `|`: the one that splits the first rows into the same number (at least
/// two) of columns most consistently. Ties go to the comma.
pub fn detect_delimiter(text: &str) -> u8 {
    let mut end = text.len().min(64 * 1024);
    while end > 0 && !text.is_char_boundary(end) {
        end -= 1;
    }
    let head = text.get(..end).unwrap_or("");
    let mut best = (b',', 0f64);
    for d in *b",;\t|" {
        let mut rdr = reader(head, d);
        let mut counts: Vec<usize> = Vec::new();
        let mut rec = csv::StringRecord::new();
        while counts.len() < 30 {
            match rdr.read_record(&mut rec) {
                Ok(true) => counts.push(rec.len()),
                _ => break,
            }
        }
        let Some(&first) = counts.first() else {
            continue;
        };
        if first < 2 {
            continue;
        }
        let same = counts.iter().filter(|c| **c == first).count();
        let score = first as f64 * (same as f64 / counts.len() as f64);
        if score > best.1 {
            best = (d, score);
        }
    }
    best.0
}

/// Does the text stop in the middle of a quoted cell (a cut-off file)?
pub fn ends_inside_quotes(text: &str, delim: u8) -> bool {
    #[derive(PartialEq, Clone, Copy)]
    enum S {
        Start,
        Field,
        Quoted,
        QuoteInQuoted,
    }
    let mut s = S::Start;
    for &b in text.as_bytes() {
        s = match s {
            S::Start => match b {
                b'"' => S::Quoted,
                b'\n' | b'\r' => S::Start,
                d if d == delim => S::Start,
                _ => S::Field,
            },
            S::Field => match b {
                b'\n' | b'\r' => S::Start,
                d if d == delim => S::Start,
                _ => S::Field,
            },
            S::Quoted => {
                if b == b'"' {
                    S::QuoteInQuoted
                } else {
                    S::Quoted
                }
            }
            S::QuoteInQuoted => match b {
                b'"' => S::Quoted,
                b'\n' | b'\r' => S::Start,
                d if d == delim => S::Start,
                _ => S::Field,
            },
        };
    }
    s == S::Quoted
}

/// A CSV file read row by row. The header row is read on creation.
pub struct Csv<'a> {
    rdr: csv::Reader<&'a [u8]>,
    rec: csv::StringRecord,
    pub delimiter: u8,
    pub headers: Vec<String>,
    /// The first row exactly as written (not trimmed), for formats that have no header row.
    pub first_raw: Vec<String>,
    rows: usize,
}

impl<'a> Csv<'a> {
    /// Open a table; `delimiter` is detected when `None`. A cut-off file (unclosed quote) is an
    /// error, as is one without a header row or with more than 1000 columns.
    pub fn open(text: &'a str, delimiter: Option<u8>) -> Result<Self, StorageError> {
        let delimiter = delimiter.unwrap_or_else(|| detect_delimiter(text));
        if ends_inside_quotes(text, delimiter) {
            return Err(unsupported(MSG_UNCLOSED_QUOTE));
        }
        let mut rdr = reader(text, delimiter);
        let mut rec = csv::StringRecord::new();
        let got = rdr
            .read_record(&mut rec)
            .map_err(|_| unsupported(MSG_DAMAGED))?;
        if !got {
            return Err(unsupported(super::limits::MSG_NO_ROWS));
        }
        if rec.len() > MAX_CSV_COLUMNS {
            return Err(unsupported(MSG_DAMAGED));
        }
        let headers = rec.iter().map(|h| h.trim().to_string()).collect();
        let first_raw = rec.iter().map(str::to_string).collect();
        Ok(Self {
            rdr,
            rec,
            delimiter,
            headers,
            first_raw,
            rows: 0,
        })
    }

    /// The next data row (rows with no content at all are skipped by the reader).
    pub fn next_row(&mut self) -> Result<Option<&csv::StringRecord>, StorageError> {
        loop {
            match self.rdr.read_record(&mut self.rec) {
                Ok(true) => {
                    if self.rec.iter().all(|c| c.trim().is_empty()) {
                        continue;
                    }
                    self.rows += 1;
                    if self.rows > MAX_ENTRIES {
                        return Err(unsupported(MSG_TOO_MANY));
                    }
                    return Ok(Some(&self.rec));
                }
                Ok(false) => return Ok(None),
                Err(_) => return Err(unsupported(MSG_DAMAGED)),
            }
        }
    }
}

/// The delimiter and the first row of a table (also for a file that was only partly read), or
/// `None` when it is not a table. Used by the format sniffer.
pub fn header_row(text: &str) -> Option<(u8, Vec<String>)> {
    let delim = detect_delimiter(text);
    let mut rdr = reader(text, delim);
    let mut rec = csv::StringRecord::new();
    match rdr.read_record(&mut rec) {
        Ok(true) if rec.len() >= 2 && rec.len() <= MAX_CSV_COLUMNS => {
            Some((delim, rec.iter().map(|h| h.trim().to_string()).collect()))
        }
        _ => None,
    }
}

/// Headers and the first rows for the column mapper; every cell is cut to 80 characters.
pub struct CsvSample {
    pub delimiter: char,
    pub headers: Vec<String>,
    pub sample: Vec<Vec<String>>,
}

pub fn inspect(bytes: &[u8]) -> Result<CsvSample, StorageError> {
    let text = decode(bytes)?;
    let mut csv = Csv::open(text.as_str(), None)?;
    if csv.headers.len() < 2 {
        return Err(unsupported(MSG_NOT_TEXT));
    }
    let headers: Vec<String> = csv
        .headers
        .iter()
        .map(|h| cap_chars(h, SAMPLE_CELL_CHARS))
        .collect();
    let mut sample = Vec::new();
    while sample.len() < SAMPLE_ROWS {
        let Some(row) = csv.next_row()? else { break };
        sample.push(
            row.iter()
                .map(|c| cap_chars(c, SAMPLE_CELL_CHARS))
                .collect(),
        );
    }
    Ok(CsvSample {
        delimiter: char::from(csv.delimiter),
        headers,
        sample,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rows(text: &str) -> Vec<Vec<String>> {
        let mut csv = Csv::open(text, None).unwrap();
        let mut out = vec![csv.headers.clone()];
        while let Some(r) = csv.next_row().unwrap() {
            out.push(r.iter().map(str::to_string).collect());
        }
        out
    }

    #[test]
    fn quotes_doubled_quotes_crlf_and_multiline_cells() {
        let r = rows("a,b\r\n\"x, \"\"y\"\"\",\"line1\nline2\"\r\n");
        assert_eq!(r, vec![vec!["a", "b"], vec!["x, \"y\"", "line1\nline2"]]);
    }

    #[test]
    fn lf_cr_and_mixed_line_ends() {
        assert_eq!(rows("a,b\n1,2\n3,4").len(), 3);
        assert_eq!(rows("a,b\r1,2\r3,4\r").len(), 3);
        assert_eq!(rows("a,b\r\n1,2\n3,4\r\n").len(), 3);
    }

    #[test]
    fn blank_rows_are_skipped() {
        assert_eq!(rows("a,b\n\n1,2\n,\n \n3,4\n").len(), 3);
    }

    #[test]
    fn delimiters_are_detected() {
        assert_eq!(detect_delimiter("a,b,c\n1,2,3\n"), b',');
        assert_eq!(detect_delimiter("a;b;c\n1;2;3\n"), b';');
        assert_eq!(detect_delimiter("a\tb\tc\n1\t2\t3\n"), b'\t');
        assert_eq!(detect_delimiter("a|b|c\n1|2|3\n"), b'|');
        // commas inside the cells do not win over the real separator
        assert_eq!(
            detect_delimiter("name;note\nx;hello, world, again\ny;a, b, c\n"),
            b';'
        );
        assert_eq!(detect_delimiter("single column\nonly\n"), b',');
        assert_eq!(detect_delimiter(""), b',');
        assert_eq!(rows("name;pw\nx;y\n")[1], vec!["x", "y"]);
    }

    #[test]
    fn a_cut_off_file_is_reported() {
        assert!(Csv::open("a,b\n1,\"cut off", None).is_err());
        assert!(Csv::open("a,b\n1,\"fine\"\n", None).is_ok());
        assert!(
            Csv::open("a,b\n1,5\" pipe\n", None).is_ok(),
            "a quote inside an unquoted cell is text"
        );
        assert!(ends_inside_quotes("\"abc", b','));
        assert!(!ends_inside_quotes("\"abc\"", b','));
        assert!(!ends_inside_quotes("\"a\"\"b\"", b','));
    }

    #[test]
    fn byte_order_marks_and_utf16() {
        let bom = [&[0xEF, 0xBB, 0xBF][..], b"name,pw\nx,y\n"].concat();
        assert_eq!(decode(&bom).unwrap().as_str(), "name,pw\nx,y\n");
        let mut utf16: Vec<u8> = vec![0xFF, 0xFE];
        for u in "name,pw\nx,é\n".encode_utf16() {
            utf16.extend(u.to_le_bytes());
        }
        assert_eq!(decode(&utf16).unwrap().as_str(), "name,pw\nx,é\n");
        let mut be: Vec<u8> = vec![0xFE, 0xFF];
        for u in "ab".encode_utf16() {
            be.extend(u.to_be_bytes());
        }
        assert_eq!(decode(&be).unwrap().as_str(), "ab");
    }

    #[test]
    fn rare_invalid_utf8_is_repaired_but_binary_is_refused() {
        let mut ok = b"name,note\nx,caf".to_vec();
        ok.push(0xE9); // latin-1 e-acute
        ok.extend(b" au lait and a lot more ordinary text to stay mostly fine\n");
        let t = decode(&ok).unwrap();
        assert!(t.as_str().contains("caf\u{FFFD} au lait"));
        assert!(decode(&[0u8; 20]).is_err());
        assert!(decode(b"abc\0def").is_err());
        assert!(decode(&[0x80; 50]).is_err());
    }

    #[test]
    fn inspect_caps_cells_and_counts_rows() {
        let long = "x".repeat(300);
        let text = format!("a,b\n1,{long}\n2,3\n3,3\n4,3\n5,3\n6,3\n7,3\n");
        let s = inspect(text.as_bytes()).unwrap();
        assert_eq!(s.delimiter, ',');
        assert_eq!(s.headers, vec!["a", "b"]);
        assert_eq!(s.sample.len(), SAMPLE_ROWS);
        assert_eq!(s.sample[0][1].chars().count(), SAMPLE_CELL_CHARS);
        assert!(inspect(b"just one column\nvalue\n").is_err());
        assert!(inspect(b"").is_err());
    }

    #[test]
    fn header_normalisation() {
        assert_eq!(norm("Login_URI"), "loginuri");
        assert_eq!(norm(" One-time password "), "onetimepassword");
    }
}
