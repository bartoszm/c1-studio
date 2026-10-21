//! The photo library: every source's photos in a redb store, searchable through a
//! tantivy index with per-dimension counts.

pub mod ai;
pub mod analyse;
pub mod edits;
pub mod facets;
pub mod filter;
pub mod geo;
pub mod gpx;
pub mod library;
pub mod looks;
pub mod marks;
pub mod matching;
pub mod places;
pub mod schema;
mod store;
pub mod thumbs;
pub mod trips;

pub use edits::{Event, Origin, Pending, QueuedCollection, QueuedRemap, What};
pub use facets::Counts;
pub use filter::{BBox, Bin, Filter, Hidden, Presence, Range, TimeRange, Trashed, parse_color, parse_rating};
pub use geo::{Cluster, MapResult, MapView, PlaceCluster};
pub use library::{Accept, ClassPreview, GroupSort};
pub use library::{
    FillGroup, Group, GroupPage, Library, Merged, NeighbourPlan, NeighbourSettings, PendingEdit, PlaceLookup,
    QueryOptions, QueryResult, RefinePlan, RefineSettings, RoutePoint, Selection, Sort, SyncReport, SyncStep, TripList,
    TripRow, Unresolved, group_places, new_collection_id, place_of_path, smart_filter,
};
pub use looks::Look;
pub use schema::{Dim, NO_PLACE};
