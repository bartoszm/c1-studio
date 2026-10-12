//! The photo library: every source's photos in a redb store, searchable through a
//! tantivy index with per-dimension counts.

pub mod analyse;
pub mod edits;
pub mod facets;
pub mod filter;
pub mod geo;
pub mod library;
pub mod looks;
pub mod schema;
mod store;

pub use edits::{Event, Origin, Pending, QueuedCollection, QueuedRemap};
pub use facets::Counts;
pub use filter::{BBox, Filter, Presence, Range, TimeRange, Trashed, parse_color, parse_rating};
pub use geo::{Cluster, MapResult, MapView};
pub use library::{
    Group, Library, PendingEdit, QueryOptions, QueryResult, Sort, SyncReport, new_collection_id, smart_filter,
};
pub use looks::Look;
pub use schema::{Dim, NO_PLACE};
