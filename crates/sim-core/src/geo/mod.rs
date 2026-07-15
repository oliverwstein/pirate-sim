//! Tier-1 global geodesic mesh (see `mesh.rs` for the full design note).
//!
//! This is additive: nothing in the existing flat-plane `map`, `nav`,
//! `tile_mesh`, or `coastline_geom` modules depends on this yet. It is the
//! foundation for global land/sea classification, reachability, rivers,
//! and coarse shipping-lane routing (see `planning/development-log.md`).

pub mod classification;
pub mod index;
pub mod mesh;

pub use classification::LandSea;
pub use index::DirectionIndex;
pub use mesh::{expected_tile_count, GeoMesh, TileId, DEFAULT_SUBDIVISIONS};
