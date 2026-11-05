//! Funds transfer pricing: the curves ALM publishes each day from its workbook, read by every desk.
//! A publication holds one curve per currency; which currencies is whatever ALM published.

use chrono::{DateTime, NaiveDate, Utc};
use serde::{Deserialize, Serialize};

use crate::alm::CurvePoint;

/// One currency's FTP curve, points sorted by tenor. Rates in percent.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FtpCurve {
    pub ccy: String,
    pub points: Vec<CurvePoint>,
}

/// A stored publication.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FtpPublication {
    pub id: i32,
    pub as_of: NaiveDate,
    pub published_at: DateTime<Utc>,
    /// Display name of the publisher; `None` when the reconciler wrote it (synthetic data).
    pub published_by: Option<String>,
    pub file_name: Option<String>,
    pub note: Option<String>,
    /// Earlier publications for the same day that this one replaced.
    pub superseded: i64,
    pub curves: Vec<FtpCurve>,
}

/// A publication without its points, for the history list.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FtpPublicationSummary {
    pub id: i32,
    pub as_of: NaiveDate,
    pub published_at: DateTime<Utc>,
    pub published_by: Option<String>,
    pub file_name: Option<String>,
    pub note: Option<String>,
    pub currencies: Vec<String>,
    pub is_current: bool,
}

/// What a desk sees: the publication in force on a date, the one before it, and how old it is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FtpCurrent {
    /// The date asked about (the business date when none was given).
    pub date: NaiveDate,
    pub current: Option<FtpPublication>,
    pub previous: Option<FtpPublication>,
    /// Business days between `current.as_of` and `date`; 0 when published for that day.
    pub age_days: Option<i64>,
    /// True when the curves in force were not published for `date` itself.
    pub stale: bool,
    pub can_publish: bool,
    pub recent: Vec<FtpPublicationSummary>,
}

/// One tenor's rate over time, for a chart.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FtpHistoryPoint {
    pub as_of: NaiveDate,
    pub rate: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FtpHistory {
    pub ccy: String,
    pub tenor: String,
    pub points: Vec<FtpHistoryPoint>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FtpHistoryQuery {
    pub ccy: String,
    pub tenor: String,
    #[serde(default)]
    pub days: Option<i64>,
}

/// A problem found while reading the workbook, pointing at where it is.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SheetIssue {
    pub sheet: String,
    /// Excel reference such as `C7`; empty when the issue is about the whole sheet.
    pub cell: String,
    pub message: String,
}

/// How a curve differs from the publication in force before it.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum FtpChange {
    /// A currency with no curve in the previous publication.
    Added { ccy: String },
    /// A currency in the previous publication that this one leaves out: it is retired.
    Missing { ccy: String },
    /// A point that moved by more than the warning threshold.
    Moved { ccy: String, tenor: String, from: f64, to: f64 },
    /// Tenors the previous curve had and this one does not, or the reverse.
    Tenors { ccy: String, added: Vec<String>, removed: Vec<String> },
}

/// The result of reading a workbook, before anything is stored.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FtpPreview {
    /// The date found in the workbook, if it has one.
    pub sheet_date: Option<NaiveDate>,
    /// The date the publication will carry: the workbook's, else the business date.
    pub as_of: NaiveDate,
    pub layout: String,
    pub curves: Vec<FtpCurve>,
    /// The workbook gave rates as fractions (0.0525) and they were read as percent (5.25).
    pub fractions: bool,
    pub errors: Vec<SheetIssue>,
    pub warnings: Vec<SheetIssue>,
    pub changes: Vec<FtpChange>,
    /// The publication the changes are measured against, and its curves.
    pub compared_with: Option<NaiveDate>,
    #[serde(default)]
    pub previous: Vec<FtpCurve>,
    pub file_sha256: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct FtpPublishRequest {
    pub as_of: NaiveDate,
    pub curves: Vec<FtpCurve>,
    #[serde(default)]
    pub file_name: Option<String>,
    #[serde(default)]
    pub file_sha256: Option<String>,
    pub note: String,
}
