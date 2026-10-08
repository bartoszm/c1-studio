//! The selection flags, spelled as the Python `c1cat` spelled them so the two can be
//! compared command for command.

use anyhow::{Context, bail};
use c1index::{BBox, Filter, Library, Range, TimeRange, Trashed, parse_color, parse_rating};
use chrono::{Datelike, Local, NaiveDate, TimeZone};
use clap::Args;

#[derive(Args, Debug, Default, Clone)]
pub struct Selector {
    /// Variant id (repeatable).
    #[arg(long = "id")]
    pub ids: Vec<String>,
    /// Image (asset) id (repeatable).
    #[arg(long = "image-id")]
    pub image_ids: Vec<String>,
    /// Filename glob, e.g. 'DSCF*.RAF'.
    #[arg(long)]
    pub filename: Option<String>,
    /// Comma-separated extensions, e.g. raf,dng.
    #[arg(long)]
    pub ext: Option<String>,
    /// Folder subtree, e.g. 'H:\fotos\2026'.
    #[arg(long)]
    pub folder: Option<String>,
    /// '>=4', '3', '2-4', 'none' or 'any'.
    #[arg(long)]
    pub rating: Option<String>,
    /// Colour name(s), 'none' or 'any'.
    #[arg(long, alias = "colour")]
    pub color: Option<String>,
    /// Keyword path; repeat to require all.
    #[arg(long = "keyword")]
    pub keywords: Vec<String>,
    /// Comma-separated keywords, match any.
    #[arg(long)]
    pub keyword_any: Option<String>,
    /// Keyword and everything below it (repeatable).
    #[arg(long)]
    pub keyword_under: Vec<String>,
    /// Exclude this keyword (repeatable).
    #[arg(long)]
    pub no_keyword: Vec<String>,
    #[arg(long)]
    pub unkeyworded: bool,
    /// Camera model, substring.
    #[arg(long)]
    pub camera: Option<String>,
    /// Lens, substring.
    #[arg(long)]
    pub lens: Option<String>,
    #[arg(long)]
    pub iso: Option<String>,
    #[arg(long)]
    pub aperture: Option<String>,
    #[arg(long)]
    pub focal: Option<String>,
    /// Capture date from (YYYY-MM-DD, YYYY-MM or YYYY).
    #[arg(long = "from")]
    pub date_from: Option<String>,
    /// Capture date to, inclusive.
    #[arg(long = "to")]
    pub date_to: Option<String>,
    #[arg(long)]
    pub year: Option<i32>,
    /// YYYY-MM.
    #[arg(long)]
    pub month: Option<String>,
    #[arg(long)]
    pub imported_from: Option<String>,
    #[arg(long)]
    pub imported_to: Option<String>,
    /// Album name or #id (repeatable).
    #[arg(long)]
    pub album: Vec<String>,
    #[arg(long)]
    pub not_in_album: Vec<String>,
    /// has | none.
    #[arg(long)]
    pub gps: Option<String>,
    /// LAT,LON,KM.
    #[arg(long)]
    pub near: Option<String>,
    #[arg(long)]
    pub city: Option<String>,
    #[arg(long)]
    pub country: Option<String>,
    #[arg(long)]
    pub state: Option<String>,
    /// Free text over keywords, captions, IPTC and filename.
    #[arg(long)]
    pub text: Option<String>,
    #[arg(long)]
    pub edited: bool,
    #[arg(long)]
    pub unedited: bool,
    #[arg(long)]
    pub style: Option<String>,
    #[arg(long)]
    pub has_local_layers: bool,
    #[arg(long)]
    pub include_trashed: bool,
    #[arg(long)]
    pub only_trashed: bool,
    #[arg(long)]
    pub variant_index: Option<u32>,
}

fn split(list: &str) -> Vec<String> {
    list.split(',').map(str::trim).filter(|s| !s.is_empty()).map(str::to_owned).collect()
}

/// Local-time epoch milliseconds for the start, or the last millisecond, of a day,
/// month or year — what the Python `_epoch` computed with `time.mktime`.
pub fn local_bound(text: &str, end: bool) -> anyhow::Result<i64> {
    let parts: Vec<&str> = text.trim().split('-').collect();
    let num = |s: &str| s.parse::<u32>().with_context(|| format!("cannot parse date {text:?}, use YYYY-MM-DD"));
    let (year, month, day) = match parts.as_slice() {
        [y] => (num(y)? as i32, None, None),
        [y, m] => (num(y)? as i32, Some(num(m)?), None),
        [y, m, d] => (num(y)? as i32, Some(num(m)?), Some(num(d)?)),
        _ => bail!("cannot parse date {text:?}, use YYYY-MM-DD"),
    };
    let start = NaiveDate::from_ymd_opt(year, month.unwrap_or(1), day.unwrap_or(1))
        .with_context(|| format!("no such date {text:?}"))?;
    let date = if !end {
        start
    } else {
        let next = match (month, day) {
            (Some(_), Some(_)) => start.succ_opt(),
            (Some(m), None) => if m == 12 { NaiveDate::from_ymd_opt(year + 1, 1, 1) } else { NaiveDate::from_ymd_opt(year, m + 1, 1) },
            _ => NaiveDate::from_ymd_opt(start.year() + 1, 1, 1),
        };
        next.context("date out of range")?
    };
    let local = Local
        .from_local_datetime(&date.and_hms_opt(0, 0, 0).unwrap())
        .earliest()
        .context("local time does not exist")?;
    Ok(local.timestamp_millis() - if end { 1000 } else { 0 })
}

impl Selector {
    pub fn to_filter(&self, library: &Library) -> anyhow::Result<Filter> {
        let mut f = Filter {
            ids: self.ids.clone(),
            assets: self.image_ids.clone(),
            filename: self.filename.clone(),
            ext: self.ext.as_deref().map(split).unwrap_or_default(),
            folder: self.folder.clone(),
            rating: self.rating.as_deref().map(parse_rating).transpose()?,
            color: self.color.as_deref().map(parse_color).transpose()?,
            keywords: self.keywords.clone(),
            keywords_any: self.keyword_any.as_deref().map(split).unwrap_or_default(),
            keywords_under: self.keyword_under.clone(),
            keywords_not: self.no_keyword.clone(),
            unkeyworded: self.unkeyworded,
            camera: self.camera.clone(),
            lens: self.lens.clone(),
            iso: self.iso.as_deref().map(Range::parse).transpose()?,
            aperture: self.aperture.as_deref().map(Range::parse).transpose()?,
            focal: self.focal.as_deref().map(Range::parse).transpose()?,
            year: self.year,
            month: self.month.clone(),
            city: self.city.clone(),
            state: self.state.clone(),
            country: self.country.clone(),
            text: self.text.clone(),
            style: self.style.clone(),
            has_local_layers: self.has_local_layers,
            variant_index: self.variant_index,
            ..Filter::default()
        };
        if self.date_from.is_some() || self.date_to.is_some() {
            f.captured = Some(TimeRange {
                from: self.date_from.as_deref().map(|d| local_bound(d, false)).transpose()?,
                to: self.date_to.as_deref().map(|d| local_bound(d, true)).transpose()?,
            });
        }
        if self.imported_from.is_some() || self.imported_to.is_some() {
            f.imported = Some(TimeRange {
                from: self.imported_from.as_deref().map(|d| local_bound(d, false)).transpose()?,
                to: self.imported_to.as_deref().map(|d| local_bound(d, true)).transpose()?,
            });
        }
        for name in &self.album {
            f.collections.push(library.resolve_collection(name)?);
        }
        for name in &self.not_in_album {
            f.not_in_collections.push(library.resolve_collection(name)?);
        }
        f.gps = match self.gps.as_deref() {
            None => None,
            Some("has") => Some(true),
            Some("none") => Some(false),
            Some(other) => bail!("--gps expects has or none, not {other:?}"),
        };
        if let Some(near) = &self.near {
            let parts: Vec<f64> = near.split(',').map(|p| p.trim().parse()).collect::<Result<_, _>>().context("--near expects LAT,LON,KM")?;
            let [lat, lon, km] = parts.as_slice() else { bail!("--near expects LAT,LON,KM") };
            f.within = Some(BBox::around(*lat, *lon, *km));
        }
        f.edited = match (self.edited, self.unedited) {
            (true, false) => Some(true),
            (false, true) => Some(false),
            (true, true) => bail!("--edited and --unedited exclude each other"),
            (false, false) => None,
        };
        f.trashed = match (self.include_trashed, self.only_trashed) {
            (_, true) => Trashed::Only,
            (true, false) => Trashed::Include,
            _ => Trashed::Exclude,
        };
        Ok(f)
    }
}
