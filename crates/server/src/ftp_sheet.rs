//! Reads ALM's FTP workbook, and writes the template ALM can start the next one from.
//!
//! Two layouts are understood, so the bank's existing sheet works without reformatting:
//! - **Wide**: a header row with a `Tenor` cell and one column per currency, headed by its code
//!   (`BHD`, `USD`, `AED (%)`, ...). One sheet holds every curve.
//! - **One sheet per currency**: the sheet is named by the currency code, with a `Tenor` column and
//!   the rate in the next column.
//!
//! A cell labelled `As of` or `Date` gives the publication date. Tenors read as `ON`, `1W`, `3M`,
//! `5Y`, `3 months`, `1 year`, or a plain number of days. Rates are percent; a workbook whose rates
//! are all below 0.3 is taken to hold fractions (0.0525) and read as percent, and the preview says
//! so. Every problem names its sheet and cell.

use std::collections::{BTreeMap, BTreeSet};
use std::io::Cursor;

use calamine::{Data, DataType, Range, Reader, open_workbook_auto_from_rs};
use chrono::{Datelike, NaiveDate};
use domain::{CurvePoint, FtpChange, FtpCurve, SheetIssue};

/// A point moving more than this between two publications is called out in the preview.
pub const MOVE_WARN_BP: f64 = 25.0;

/// Active ISO 4217 codes, so a column headed `Mid` or `Bid` is not taken for a currency.
const ISO: &str = "AED AFN ALL AMD ANG AOA ARS AUD AWG AZN BAM BBD BDT BGN BHD BIF BMD BND BOB BRL BSD BTN BWP BYN BZD \
CAD CDF CHF CLP CNY COP CRC CUP CVE CZK DJF DKK DOP DZD EGP ERN ETB EUR FJD FKP GBP GEL GHS GIP GMD GNF GTQ GYD HKD HNL \
HTG HUF IDR ILS INR IQD IRR ISK JMD JOD JPY KES KGS KHR KMF KPW KRW KWD KYD KZT LAK LBP LKR LRD LSL LYD MAD MDL MGA MKD \
MMK MNT MOP MRU MUR MVR MWK MXN MYR MZN NAD NGN NIO NOK NPR NZD OMR PAB PEN PGK PHP PKR PLN PYG QAR RON RSD RUB RWF SAR \
SBD SCR SDG SEK SGD SHP SLE SOS SRD SSP STN SVC SYP SZL THB TJS TMT TND TOP TRY TTD TWD TZS UAH UGX USD UYU UZS VES VND \
VUV WST XAF XCD XOF XPF YER ZAR ZMW ZWL";

pub fn is_currency(code: &str) -> bool {
    code.len() == 3 && ISO.split(' ').any(|c| c == code)
}

/// What a workbook held.
#[derive(Debug, Default)]
pub struct Parsed {
    pub sheet_date: Option<NaiveDate>,
    pub layout: String,
    pub curves: Vec<FtpCurve>,
    pub fractions: bool,
    pub errors: Vec<SheetIssue>,
    pub warnings: Vec<SheetIssue>,
}

fn issue(sheet: &str, cell: &str, message: impl Into<String>) -> SheetIssue {
    SheetIssue { sheet: sheet.to_string(), cell: cell.to_string(), message: message.into() }
}

/// `C7` from absolute zero-based row and column.
pub fn cell_ref(row: u32, col: u32) -> String {
    let mut n = col + 1;
    let mut letters = Vec::new();
    while n > 0 {
        let r = (n - 1) % 26;
        letters.push(b'A' + r as u8);
        n = (n - 1) / 26;
    }
    letters.reverse();
    format!("{}{}", String::from_utf8(letters).unwrap_or_default(), row + 1)
}

/// Canonical label and days for a tenor cell, or `None` when it is not one.
pub fn parse_tenor(cell: &Data) -> Option<(String, i32)> {
    match cell {
        Data::Int(n) if *n > 0 && *n <= 36_500 => Some((format!("{n}D"), *n as i32)),
        Data::Float(f) if *f > 0.0 && f.fract() == 0.0 && *f <= 36_500.0 => Some((format!("{}D", *f as i64), *f as i32)),
        Data::String(s) => parse_tenor_text(s),
        _ => None,
    }
}

pub fn parse_tenor_text(raw: &str) -> Option<(String, i32)> {
    let s: String = raw.trim().to_uppercase().chars().filter(|c| !c.is_whitespace() && *c != '/' && *c != '-' && *c != '.').collect();
    match s.as_str() {
        "ON" | "OVERNIGHT" => return Some(("ON".into(), 1)),
        "TN" | "TOMNEXT" => return Some(("TN".into(), 2)),
        "SN" | "SPOTNEXT" => return Some(("SN".into(), 3)),
        _ => {}
    }
    let split = s.find(|c: char| !c.is_ascii_digit())?;
    let (num, unit) = s.split_at(split);
    let n: i32 = num.parse().ok().filter(|n| *n > 0)?;
    let unit = unit.trim_end_matches('S');
    let (label, days) = match unit {
        "D" | "DAY" => (format!("{n}D"), n),
        "W" | "WK" | "WEEK" => (format!("{n}W"), n * 7),
        "M" | "MO" | "MTH" | "MONTH" => (format!("{n}M"), (n as f64 * 365.0 / 12.0).floor() as i32),
        "Y" | "YR" | "YEAR" => (format!("{n}Y"), n * 365),
        _ => return None,
    };
    (days <= 36_500).then_some((label, days))
}

fn text(cell: &Data) -> Option<String> {
    match cell {
        Data::String(s) => Some(s.trim().to_string()),
        _ => None,
    }
}

/// A rate cell: `Some(Ok(v))` a number, `Some(Err(..))` something that is not one, `None` empty.
/// A trailing `%` in text marks the value as already in percent.
fn rate(cell: &Data) -> Option<Result<(f64, bool), String>> {
    match cell {
        Data::Empty => None,
        Data::Int(n) => Some(Ok((*n as f64, false))),
        Data::Float(f) if f.is_finite() => Some(Ok((*f, false))),
        Data::String(s) if s.trim().is_empty() => None,
        Data::String(s) => {
            let t = s.trim();
            let pct = t.ends_with('%');
            match t.trim_end_matches('%').trim().replace(',', "").parse::<f64>() {
                Ok(v) if v.is_finite() => Some(Ok((v, pct))),
                _ => Some(Err(format!("'{t}' is not a rate"))),
            }
        }
        Data::Error(e) => Some(Err(format!("the cell holds an Excel error ({e:?})"))),
        other => Some(Err(format!("'{other}' is not a rate"))),
    }
}

fn date_of(cell: &Data) -> Option<NaiveDate> {
    if let Some(d) = cell.as_date() {
        return Some(d);
    }
    if let Data::String(s) = cell {
        let t = s.trim();
        for f in ["%Y-%m-%d", "%d/%m/%Y", "%d-%m-%Y", "%d-%b-%Y", "%d %b %Y", "%d %B %Y", "%d-%b-%y"] {
            if let Ok(d) = NaiveDate::parse_from_str(t, f) {
                return Some(d);
            }
        }
    }
    None
}

fn is_date_label(s: &str) -> bool {
    let k = s.trim().trim_end_matches(':').trim().to_lowercase();
    matches!(k.as_str(), "as of" | "as-of" | "asof" | "date" | "as of date" | "value date" | "curve date" | "ftp date" | "business date")
}

/// The currency a column header names: `BHD`, `bhd`, `USD (%)`, `AED FTP`.
fn header_ccy(s: &str) -> Option<String> {
    let t = s.trim().to_uppercase();
    let head: String = t.chars().take_while(|c| c.is_ascii_alphabetic()).collect();
    (head.len() == 3 && is_currency(&head)).then_some(head)
}

struct Sheet {
    name: String,
    range: Range<Data>,
    r0: u32,
    c0: u32,
}

impl Sheet {
    fn rows(&self) -> usize {
        self.range.height()
    }
    fn cols(&self) -> usize {
        self.range.width()
    }
    fn at(&self, r: usize, c: usize) -> &Data {
        static EMPTY: Data = Data::Empty;
        self.range.get((r, c)).unwrap_or(&EMPTY)
    }
    fn cell(&self, r: usize, c: usize) -> String {
        cell_ref(self.r0 + r as u32, self.c0 + c as u32)
    }
    fn find_tenor_header(&self) -> Option<(usize, usize)> {
        (0..self.rows()).find_map(|r| (0..self.cols()).find(|&c| text(self.at(r, c)).is_some_and(|s| s.eq_ignore_ascii_case("tenor"))).map(|c| (r, c)))
    }
    fn find_date(&self) -> Option<(NaiveDate, String)> {
        for r in 0..self.rows() {
            for c in 0..self.cols() {
                if text(self.at(r, c)).is_some_and(|s| is_date_label(&s)) {
                    for (rr, cc) in [(r, c + 1), (r + 1, c)] {
                        if let Some(d) = date_of(self.at(rr, cc)) {
                            return Some((d, self.cell(rr, cc)));
                        }
                    }
                }
            }
        }
        None
    }
}

/// Raw points before the fractions check: (ccy, tenor, days, value, already percent, where).
type Raw = (String, String, i32, f64, bool, String, String);

/// Reads one curve column below a header row until the tenors run out.
fn read_column(sh: &Sheet, header_row: usize, tenor_col: usize, rate_col: usize, ccy: &str, out: &mut Vec<Raw>, p: &mut Parsed) {
    let mut seen = BTreeSet::new();
    let mut blank_run = 0;
    for r in header_row + 1..sh.rows() {
        let tcell = sh.at(r, tenor_col);
        if matches!(tcell, Data::Empty) || text(tcell).is_some_and(|s| s.is_empty()) {
            blank_run += 1;
            if blank_run >= 2 {
                break;
            }
            continue;
        }
        blank_run = 0;
        let Some((tenor, days)) = parse_tenor(tcell) else {
            // A note or a footer below the table ends it; anything else in a tenor cell is an error.
            if text(tcell).is_some_and(|s| s.len() > 12) {
                break;
            }
            p.errors.push(issue(&sh.name, &sh.cell(r, tenor_col), format!("'{tcell}' is not a tenor (use ON, 1W, 3M, 5Y or a number of days)")));
            continue;
        };
        match rate(sh.at(r, rate_col)) {
            None => p.warnings.push(issue(&sh.name, &sh.cell(r, rate_col), format!("no {ccy} rate for {tenor}; that point is left out"))),
            Some(Err(e)) => p.errors.push(issue(&sh.name, &sh.cell(r, rate_col), format!("{ccy} {tenor}: {e}"))),
            Some(Ok((v, pct))) => {
                if !seen.insert(days) {
                    p.errors.push(issue(&sh.name, &sh.cell(r, tenor_col), format!("{ccy} has {tenor} twice")));
                    continue;
                }
                out.push((ccy.to_string(), tenor, days, v, pct, sh.name.clone(), sh.cell(r, rate_col)));
            }
        }
    }
}

pub fn parse(bytes: &[u8]) -> Parsed {
    let mut p = Parsed::default();
    let mut wb = match open_workbook_auto_from_rs(Cursor::new(bytes.to_vec())) {
        Ok(wb) => wb,
        Err(_) => {
            p.errors.push(issue("", "", "the file is not an Excel workbook this reader can open (save it as .xlsx)"));
            return p;
        }
    };
    let mut sheets = Vec::new();
    for name in wb.sheet_names() {
        if let Ok(range) = wb.worksheet_range(&name) {
            let (r0, c0) = range.start().unwrap_or((0, 0));
            if !range.is_empty() {
                sheets.push(Sheet { name, range, r0, c0 });
            }
        }
    }
    if sheets.is_empty() {
        p.errors.push(issue("", "", "the workbook has no sheet with any content"));
        return p;
    }

    for sh in &sheets {
        if let Some((d, at)) = sh.find_date() {
            match p.sheet_date {
                None => p.sheet_date = Some(d),
                Some(prev) if prev != d => p.errors.push(issue(&sh.name, &at, format!("this sheet is dated {d}, another {prev}"))),
                _ => {}
            }
        }
    }

    let mut raw: Vec<Raw> = Vec::new();
    let mut where_ccy: BTreeMap<String, String> = BTreeMap::new();
    let mut layouts = BTreeSet::new();
    for sh in &sheets {
        let Some((hr, tc)) = sh.find_tenor_header() else { continue };
        let mut found = false;
        let mut ignored = Vec::new();
        for c in tc + 1..sh.cols() {
            let Some(h) = text(sh.at(hr, c)).filter(|h| !h.is_empty()) else { continue };
            let Some(ccy) = header_ccy(&h) else {
                ignored.push(issue(&sh.name, &sh.cell(hr, c), format!("column '{h}' is not headed by a currency code; it is ignored")));
                continue;
            };
            if let Some(other) = where_ccy.insert(ccy.clone(), format!("{} {}", sh.name, sh.cell(hr, c))) {
                p.errors.push(issue(&sh.name, &sh.cell(hr, c), format!("{ccy} appears twice (also at {other})")));
                continue;
            }
            read_column(sh, hr, tc, c, &ccy, &mut raw, &mut p);
            found = true;
            layouts.insert("one sheet, a column per currency");
        }
        if found {
            p.warnings.append(&mut ignored);
        }
        // One sheet per currency: the sheet's name is the currency; the rate is the next column.
        if !found && let Some(ccy) = header_ccy(&sh.name).filter(|_| sh.name.trim().len() <= 12) {
            if let Some(other) = where_ccy.insert(ccy.clone(), sh.name.clone()) {
                p.errors.push(issue(&sh.name, "", format!("{ccy} appears twice (also at {other})")));
                continue;
            }
            read_column(sh, hr, tc, tc + 1, &ccy, &mut raw, &mut p);
            layouts.insert("a sheet per currency");
        }
    }
    p.layout = layouts.into_iter().collect::<Vec<_>>().join(" and ");

    if raw.is_empty() {
        p.errors.push(issue(
            "",
            "",
            "no curve was found: the workbook needs a header row with a 'Tenor' cell and currency codes (BHD, USD, ...) as column headers, or one sheet per currency named by its code",
        ));
        return p;
    }

    // Fractions: every value without a % sign small enough that it cannot be a percent rate.
    let plain: Vec<f64> = raw.iter().filter(|x| !x.4).map(|x| x.3.abs()).collect();
    p.fractions = !plain.is_empty() && plain.iter().all(|v| *v <= 0.3) && plain.iter().any(|v| *v > 0.0);

    let mut curves: BTreeMap<String, Vec<CurvePoint>> = BTreeMap::new();
    for (ccy, tenor, days, v, pct, sheet, cell) in raw {
        let r = if p.fractions && !pct { v * 100.0 } else { v };
        if !(-5.0..=50.0).contains(&r) {
            p.errors.push(issue(&sheet, &cell, format!("{ccy} {tenor} reads as {r:.4}%, outside -5% to 50%")));
            continue;
        }
        curves.entry(ccy).or_default().push(CurvePoint { tenor, tenor_days: days, rate: (r * 1e6).round() / 1e6 });
    }
    for (ccy, mut pts) in curves {
        pts.sort_by_key(|x| x.tenor_days);
        if pts.len() < 2 {
            p.warnings.push(issue("", "", format!("{ccy} has only {} point; a bond of any term will be charged that one rate", pts.len())));
        }
        p.curves.push(FtpCurve { ccy, points: pts });
    }
    p
}

/// How `next` differs from `prev`, for the preview.
pub fn changes(prev: &[FtpCurve], next: &[FtpCurve]) -> Vec<FtpChange> {
    let before: BTreeMap<&str, &FtpCurve> = prev.iter().map(|c| (c.ccy.as_str(), c)).collect();
    let after: BTreeMap<&str, &FtpCurve> = next.iter().map(|c| (c.ccy.as_str(), c)).collect();
    let mut out = Vec::new();
    for (ccy, c) in &after {
        let Some(b) = before.get(ccy) else {
            out.push(FtpChange::Added { ccy: c.ccy.clone() });
            continue;
        };
        let bm: BTreeMap<i32, &CurvePoint> = b.points.iter().map(|p| (p.tenor_days, p)).collect();
        let am: BTreeMap<i32, &CurvePoint> = c.points.iter().map(|p| (p.tenor_days, p)).collect();
        let added: Vec<String> = am.iter().filter(|(d, _)| !bm.contains_key(d)).map(|(_, p)| p.tenor.clone()).collect();
        let removed: Vec<String> = bm.iter().filter(|(d, _)| !am.contains_key(d)).map(|(_, p)| p.tenor.clone()).collect();
        if !added.is_empty() || !removed.is_empty() {
            out.push(FtpChange::Tenors { ccy: c.ccy.clone(), added, removed });
        }
        for (d, p) in &am {
            if let Some(q) = bm.get(d)
                && ((p.rate - q.rate) * 100.0).abs() > MOVE_WARN_BP
            {
                out.push(FtpChange::Moved { ccy: c.ccy.clone(), tenor: p.tenor.clone(), from: q.rate, to: p.rate });
            }
        }
    }
    for ccy in before.keys() {
        if !after.contains_key(ccy) {
            out.push(FtpChange::Missing { ccy: ccy.to_string() });
        }
    }
    out
}

/// The workbook ALM can download, edit and drop back: the wide layout, filled with `curves`.
pub fn template(as_of: NaiveDate, curves: &[FtpCurve]) -> Result<Vec<u8>, rust_xlsxwriter::XlsxError> {
    use rust_xlsxwriter::{ExcelDateTime, Format, Workbook};
    let mut wb = Workbook::new();
    let ws = wb.add_worksheet();
    ws.set_name("FTP")?;
    let bold = Format::new().set_bold();
    let date = Format::new().set_num_format("dd-mmm-yyyy").set_align(rust_xlsxwriter::FormatAlign::Left);
    let pct = Format::new().set_num_format("0.0000");
    let muted = Format::new().set_italic().set_font_color(rust_xlsxwriter::Color::Gray);
    ws.write_string_with_format(0, 0, "NBB funds transfer pricing curves", &bold)?;
    ws.write_string(1, 0, "As of")?;
    ws.write_datetime_with_format(1, 1, ExcelDateTime::from_ymd(as_of.year() as u16, as_of.month() as u8, as_of.day() as u8)?, &date)?;
    ws.write_string_with_format(
        2,
        0,
        "Rates in percent. Add a column headed by a currency code to publish a new curve; delete a column to retire one.",
        &muted,
    )?;
    ws.write_string_with_format(4, 0, "Tenor", &bold)?;
    let mut tenors: BTreeMap<i32, String> = BTreeMap::new();
    for c in curves {
        for p in &c.points {
            tenors.entry(p.tenor_days).or_insert_with(|| p.tenor.clone());
        }
    }
    if tenors.is_empty() {
        for (t, d) in [("ON", 1), ("1W", 7), ("1M", 30), ("3M", 91), ("6M", 182), ("1Y", 365), ("2Y", 730), ("3Y", 1095), ("5Y", 1825), ("7Y", 2555), ("10Y", 3650)] {
            tenors.insert(d, t.to_string());
        }
    }
    for (i, (_, t)) in tenors.iter().enumerate() {
        ws.write_string(5 + i as u32, 0, t)?;
    }
    for (j, c) in curves.iter().enumerate() {
        let col = 1 + j as u16;
        ws.write_string_with_format(4, col, &c.ccy, &bold)?;
        for (i, (d, _)) in tenors.iter().enumerate() {
            if let Some(p) = c.points.iter().find(|p| p.tenor_days == *d) {
                ws.write_number_with_format(5 + i as u32, col, p.rate, &pct)?;
            }
        }
    }
    ws.set_column_width(0, 12)?;
    wb.save_to_buffer()
}

#[cfg(test)]
mod tests {
    use super::*;
    use rust_xlsxwriter::Workbook;

    fn book(build: impl FnOnce(&mut Workbook)) -> Vec<u8> {
        let mut wb = Workbook::new();
        build(&mut wb);
        wb.save_to_buffer().unwrap()
    }

    #[test]
    fn tenors_read_in_any_common_spelling() {
        assert_eq!(parse_tenor_text("O/N"), Some(("ON".into(), 1)));
        assert_eq!(parse_tenor_text("3M"), Some(("3M".into(), 91)));
        assert_eq!(parse_tenor_text("3 months"), Some(("3M".into(), 91)));
        assert_eq!(parse_tenor_text("1 Year"), Some(("1Y".into(), 365)));
        assert_eq!(parse_tenor_text("2w"), Some(("2W".into(), 14)));
        assert_eq!(parse_tenor_text("12M"), Some(("12M".into(), 365)));
        assert_eq!(parse_tenor_text("Rate"), None);
        assert_eq!(parse_tenor(&Data::Float(30.0)), Some(("30D".into(), 30)));
        assert_eq!(cell_ref(6, 2), "C7");
        assert_eq!(cell_ref(0, 27), "AB1");
    }

    #[test]
    fn the_wide_layout_reads_every_currency_column() {
        let bytes = book(|wb| {
            let ws = wb.add_worksheet();
            ws.write_string(0, 0, "As of").unwrap();
            ws.write_string(0, 1, "2026-11-05").unwrap();
            for (c, h) in ["Tenor", "BHD", "USD (%)", "Mid", "AED", "SAR"].iter().enumerate() {
                ws.write_string(2, c as u16, *h).unwrap();
            }
            for (r, (t, rates)) in [("ON", [5.10, 4.30, 9.0, 4.40, 5.20]), ("3M", [5.60, 4.45, 9.0, 4.55, 5.40]), ("1Y", [5.90, 4.20, 9.0, 4.60, 5.55])].iter().enumerate() {
                ws.write_string(3 + r as u32, 0, *t).unwrap();
                for (c, v) in rates.iter().enumerate() {
                    ws.write_number(3 + r as u32, 1 + c as u16, *v).unwrap();
                }
            }
        });
        let p = parse(&bytes);
        assert!(p.errors.is_empty(), "{:?}", p.errors);
        assert_eq!(p.sheet_date, NaiveDate::from_ymd_opt(2026, 11, 5));
        let ccys: Vec<&str> = p.curves.iter().map(|c| c.ccy.as_str()).collect();
        assert_eq!(ccys, ["AED", "BHD", "SAR", "USD"]);
        assert!(p.warnings.iter().any(|w| w.cell == "D3" && w.message.contains("Mid")), "{:?}", p.warnings);
        let bhd = &p.curves[1];
        assert_eq!(bhd.points.iter().map(|x| (x.tenor.as_str(), x.rate)).collect::<Vec<_>>(), [("ON", 5.10), ("3M", 5.60), ("1Y", 5.90)]);
        assert!(!p.fractions);
    }

    #[test]
    fn a_sheet_per_currency_and_fractions_are_understood() {
        let bytes = book(|wb| {
            for (name, base) in [("BHD", 0.055), ("USD", 0.043)] {
                let ws = wb.add_worksheet();
                ws.set_name(name).unwrap();
                ws.write_string(0, 0, "Tenor").unwrap();
                ws.write_string(0, 1, "Rate").unwrap();
                for (r, t) in ["1M", "6M", "2Y"].iter().enumerate() {
                    ws.write_string(1 + r as u32, 0, *t).unwrap();
                    ws.write_number(1 + r as u32, 1, base + r as f64 * 0.001).unwrap();
                }
            }
        });
        let p = parse(&bytes);
        assert!(p.errors.is_empty(), "{:?}", p.errors);
        assert!(p.fractions);
        assert_eq!(p.layout, "a sheet per currency");
        assert_eq!(p.curves.len(), 2);
        assert!((p.curves[0].points[0].rate - 5.5).abs() < 1e-9);
        assert!((p.curves[1].points[2].rate - 4.5).abs() < 1e-9);
    }

    #[test]
    fn bad_cells_are_reported_with_their_reference() {
        let bytes = book(|wb| {
            let ws = wb.add_worksheet();
            ws.set_name("Curves").unwrap();
            ws.write_string(0, 0, "Tenor").unwrap();
            ws.write_string(0, 1, "BHD").unwrap();
            ws.write_string(1, 0, "1M").unwrap();
            ws.write_string(1, 1, "n/a").unwrap();
            ws.write_string(2, 0, "3M").unwrap();
            ws.write_number(2, 1, 75.0).unwrap();
            ws.write_string(3, 0, "3M").unwrap();
            ws.write_number(3, 1, 5.0).unwrap();
            ws.write_string(4, 0, "6M").unwrap();
        });
        let p = parse(&bytes);
        let at = |c: &str| p.errors.iter().any(|e| e.sheet == "Curves" && e.cell == c);
        assert!(at("B2"), "{:?}", p.errors);
        assert!(at("B3"), "{:?}", p.errors);
        assert!(p.warnings.iter().any(|w| w.cell == "B5"), "{:?}", p.warnings);
        assert!(parse(b"not a workbook").errors[0].message.contains("not an Excel workbook"));
    }

    #[test]
    fn the_template_reads_back_as_what_was_written() {
        let curves = vec![
            FtpCurve { ccy: "BHD".into(), points: vec![CurvePoint { tenor: "ON".into(), tenor_days: 1, rate: 5.1 }, CurvePoint { tenor: "1Y".into(), tenor_days: 365, rate: 5.9 }] },
            FtpCurve { ccy: "USD".into(), points: vec![CurvePoint { tenor: "ON".into(), tenor_days: 1, rate: 4.3 }, CurvePoint { tenor: "1Y".into(), tenor_days: 365, rate: 4.2 }] },
        ];
        let d = NaiveDate::from_ymd_opt(2026, 11, 5).unwrap();
        let p = parse(&template(d, &curves).unwrap());
        assert!(p.errors.is_empty() && p.warnings.is_empty(), "{:?} {:?}", p.errors, p.warnings);
        assert_eq!(p.sheet_date, Some(d));
        assert_eq!(p.curves, curves);
        let mut next = curves.clone();
        next[0].points[1].rate = 6.3;
        next.remove(1);
        next.push(FtpCurve { ccy: "SAR".into(), points: curves[1].points.clone() });
        let ch = changes(&curves, &next);
        assert!(ch.contains(&FtpChange::Moved { ccy: "BHD".into(), tenor: "1Y".into(), from: 5.9, to: 6.3 }));
        assert!(ch.contains(&FtpChange::Added { ccy: "SAR".into() }));
        assert!(ch.contains(&FtpChange::Missing { ccy: "USD".into() }));
    }
}
