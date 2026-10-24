//! The photo library: every source's photos in a redb store, searchable through a
//! tantivy index with per-dimension counts.

pub mod ai;
pub mod analyse;
pub mod edits;
pub mod facets;
pub mod filter;
pub mod geo;
pub mod gpx;
pub mod history;
pub mod library;
pub mod looks;
pub mod marks;
pub mod matching;
pub mod places;
pub mod schema;
pub mod sessions;
mod store;
pub mod thumbs;
pub mod trips;

pub use edits::{Before, Event, Origin, Pending, QueuedCollection, QueuedRemap, What};
pub use facets::Counts;
pub use filter::{
    BBox, Bin, Filter, Hidden, HistoryPick, Presence, Range, TimeRange, Trashed, parse_color, parse_rating,
};
pub use geo::{Cluster, MapResult, MapView, PlaceCluster};
pub use history::{Compare, EventView, LogGroup, LogKind, LogQuery, NetWhat};
pub use library::{Accept, Accepted, ClassPreview, GroupSort, ReviewRow, Seen};
pub use library::{
    FillGroup, Group, GroupPage, HistoryOff, Library, LogDays, LogPage, Merged, NeighbourPlan, NeighbourSettings,
    PendingEdit, PlaceLookup, QueryOptions, QueryResult, RoutePoint, Selection, Sort, SyncReport, SyncStep, TripList,
    TripRow, Unresolved, group_places, new_collection_id, place_of_path, smart_filter,
};
pub use looks::Look;
pub use schema::{Dim, NO_PLACE};
