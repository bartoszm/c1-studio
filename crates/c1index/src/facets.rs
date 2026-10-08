//! Counting values per dimension over the photos a query matched.
//!
//! Reads the columnar (fast) string fields directly: per segment, every value is an
//! ordinal into that segment's dictionary, so counting is an array increment per value.
//! Ordinals are turned back into strings once per segment at the end.

use std::collections::HashMap;

use tantivy::collector::{Collector, SegmentCollector};
use tantivy::columnar::StrColumn;
use tantivy::{DocId, Score, SegmentOrdinal, SegmentReader};

use crate::schema::Dim;

pub type Counts = HashMap<Dim, Vec<(String, u64)>>;

pub struct FacetCollector {
    dims: Vec<Dim>,
}

impl FacetCollector {
    pub fn new(dims: &[Dim]) -> Self {
        Self { dims: dims.to_vec() }
    }
}

pub struct FacetSegment {
    columns: Vec<(Dim, Option<StrColumn>, Vec<u32>)>,
}

impl Collector for FacetCollector {
    type Fruit = Counts;
    type Child = FacetSegment;

    fn for_segment(&self, _ord: SegmentOrdinal, segment: &SegmentReader) -> tantivy::Result<FacetSegment> {
        let mut columns = Vec::with_capacity(self.dims.len());
        for &dim in &self.dims {
            let column = segment.fast_fields().str(dim.field_name())?;
            let counts = vec![0u32; column.as_ref().map_or(0, |c| c.num_terms())];
            columns.push((dim, column, counts));
        }
        Ok(FacetSegment { columns })
    }

    fn requires_scoring(&self) -> bool {
        false
    }

    fn merge_fruits(&self, segments: Vec<Counts>) -> tantivy::Result<Counts> {
        let mut merged: HashMap<Dim, HashMap<String, u64>> = HashMap::new();
        for segment in segments {
            for (dim, values) in segment {
                let into = merged.entry(dim).or_default();
                for (value, n) in values {
                    *into.entry(value).or_default() += n;
                }
            }
        }
        Ok(self
            .dims
            .iter()
            .map(|dim| {
                let mut values: Vec<(String, u64)> = merged.remove(dim).unwrap_or_default().into_iter().collect();
                // Most frequent first, then alphabetical for a stable order.
                values.sort_by(|a, b| b.1.cmp(&a.1).then_with(|| a.0.cmp(&b.0)));
                (*dim, values)
            })
            .collect())
    }
}

impl SegmentCollector for FacetSegment {
    type Fruit = Counts;

    fn collect(&mut self, doc: DocId, _score: Score) {
        for (_, column, counts) in &mut self.columns {
            if let Some(column) = column {
                for ord in column.term_ords(doc) {
                    counts[ord as usize] += 1;
                }
            }
        }
    }

    fn harvest(self) -> Counts {
        let mut out = HashMap::new();
        for (dim, column, counts) in self.columns {
            let mut values = Vec::new();
            if let Some(column) = column {
                let mut text = String::new();
                for (ord, &n) in counts.iter().enumerate() {
                    if n > 0 && column.ord_to_str(ord as u64, &mut text).unwrap_or(false) {
                        values.push((text.clone(), u64::from(n)));
                    }
                }
            }
            out.insert(dim, values);
        }
        out
    }
}
