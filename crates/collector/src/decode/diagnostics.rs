use super::{
    AIR_BASES_COPY_NS, AIR_BASES_RAW_BYTES, AIR_STATS_COPY_NS, AIR_STATS_RAW_BYTES,
    AIR_WING_COPY_NS, AIR_WING_RAW_BYTES, CAPITULATION_COPY_NS, CAPITULATION_RAW_BYTES,
    CHARACTERS_COPY_NS, CHARACTERS_RAW_BYTES, CONSTRUCTION_REST_COPY_NS,
    CONSTRUCTION_REST_RAW_BYTES, DIVISIONS_COPY_NS, DIVISIONS_RAW_BYTES, ECONOMY_REST_COPY_NS,
    ECONOMY_REST_RAW_BYTES, INTEL_COPY_NS, INTEL_RAW_BYTES, LEXICAL_COPY_NS, LEXICAL_RAW_BYTES,
    NAVAL_COPY_NS, NAVAL_RAW_BYTES, Ordering, PLAYERS_COPY_NS, PLAYERS_RAW_BYTES,
    PRODUCTION_REST_COPY_NS, PRODUCTION_REST_RAW_BYTES, PROJECTS_COPY_NS, PROJECTS_RAW_BYTES,
    RADAR_COPY_NS, RADAR_RAW_BYTES, RELATIONS_COPY_NS, RELATIONS_RAW_BYTES, SESSION_COPY_NS,
    SESSION_RAW_BYTES, SUNK_COPY_NS, SUNK_RAW_BYTES, SUPREMACY_COPY_NS, SUPREMACY_RAW_BYTES,
    WEATHER_COPY_NS, WEATHER_RAW_BYTES,
};

pub(crate) fn add_relations_copy_ns(table: usize, ns: u64) {
    RELATIONS_COPY_NS[table].fetch_add(ns, Ordering::Relaxed);
}

pub(crate) fn add_relations_raw_bytes(table: usize, bytes: u64) {
    RELATIONS_RAW_BYTES[table].fetch_add(bytes, Ordering::Relaxed);
}

pub(crate) fn add_characters_copy_ns(table: usize, ns: u64) {
    CHARACTERS_COPY_NS[table].fetch_add(ns, Ordering::Relaxed);
}

pub(crate) fn add_characters_raw_bytes(table: usize, bytes: u64) {
    CHARACTERS_RAW_BYTES[table].fetch_add(bytes, Ordering::Relaxed);
}

pub(crate) fn add_projects_copy_ns(table: usize, ns: u64) {
    PROJECTS_COPY_NS[table].fetch_add(ns, Ordering::Relaxed);
}

pub(crate) fn add_projects_raw_bytes(table: usize, bytes: u64) {
    PROJECTS_RAW_BYTES[table].fetch_add(bytes, Ordering::Relaxed);
}

pub(crate) fn add_lexical_copy_ns(table: usize, ns: u64) {
    LEXICAL_COPY_NS[table].fetch_add(ns, Ordering::Relaxed);
}

pub(crate) fn add_lexical_raw_bytes(table: usize, bytes: u64) {
    LEXICAL_RAW_BYTES[table].fetch_add(bytes, Ordering::Relaxed);
}

pub(crate) fn add_divisions_copy_ns(table: usize, ns: u64) {
    DIVISIONS_COPY_NS[table].fetch_add(ns, Ordering::Relaxed);
}

pub(crate) fn add_divisions_raw_bytes(table: usize, bytes: u64) {
    DIVISIONS_RAW_BYTES[table].fetch_add(bytes, Ordering::Relaxed);
}

pub(crate) fn add_air_wing_copy_ns(table: usize, ns: u64) {
    AIR_WING_COPY_NS[table].fetch_add(ns, Ordering::Relaxed);
}

pub(crate) fn add_air_wing_raw_bytes(table: usize, bytes: u64) {
    AIR_WING_RAW_BYTES[table].fetch_add(bytes, Ordering::Relaxed);
}

pub(crate) fn add_air_stats_copy_ns(table: usize, ns: u64) {
    AIR_STATS_COPY_NS[table].fetch_add(ns, Ordering::Relaxed);
}

pub(crate) fn add_air_stats_raw_bytes(table: usize, bytes: u64) {
    AIR_STATS_RAW_BYTES[table].fetch_add(bytes, Ordering::Relaxed);
}

pub(crate) fn add_naval_copy_ns(table: usize, ns: u64) {
    NAVAL_COPY_NS[table].fetch_add(ns, Ordering::Relaxed);
}

pub(crate) fn add_naval_raw_bytes(table: usize, bytes: u64) {
    NAVAL_RAW_BYTES[table].fetch_add(bytes, Ordering::Relaxed);
}

pub(crate) fn add_players_copy_ns(table: usize, ns: u64) {
    PLAYERS_COPY_NS[table].fetch_add(ns, Ordering::Relaxed);
}

pub(crate) fn add_players_raw_bytes(table: usize, bytes: u64) {
    PLAYERS_RAW_BYTES[table].fetch_add(bytes, Ordering::Relaxed);
}

pub(crate) fn add_session_copy_ns(table: usize, ns: u64) {
    SESSION_COPY_NS[table].fetch_add(ns, Ordering::Relaxed);
}

pub(crate) fn add_session_raw_bytes(table: usize, bytes: u64) {
    SESSION_RAW_BYTES[table].fetch_add(bytes, Ordering::Relaxed);
}

pub(crate) fn add_production_rest_copy_ns(table: usize, ns: u64) {
    PRODUCTION_REST_COPY_NS[table].fetch_add(ns, Ordering::Relaxed);
}

pub(crate) fn add_production_rest_raw_bytes(table: usize, bytes: u64) {
    PRODUCTION_REST_RAW_BYTES[table].fetch_add(bytes, Ordering::Relaxed);
}

pub(crate) fn add_construction_rest_copy_ns(table: usize, ns: u64) {
    CONSTRUCTION_REST_COPY_NS[table].fetch_add(ns, Ordering::Relaxed);
}

pub(crate) fn add_construction_rest_raw_bytes(table: usize, bytes: u64) {
    CONSTRUCTION_REST_RAW_BYTES[table].fetch_add(bytes, Ordering::Relaxed);
}

pub(crate) fn add_economy_rest_copy_ns(table: usize, ns: u64) {
    ECONOMY_REST_COPY_NS[table].fetch_add(ns, Ordering::Relaxed);
}

pub(crate) fn add_economy_rest_raw_bytes(table: usize, bytes: u64) {
    ECONOMY_REST_RAW_BYTES[table].fetch_add(bytes, Ordering::Relaxed);
}

pub(crate) fn add_air_bases_copy_ns(table: usize, ns: u64) {
    AIR_BASES_COPY_NS[table].fetch_add(ns, Ordering::Relaxed);
}

pub(crate) fn add_air_bases_raw_bytes(table: usize, bytes: u64) {
    AIR_BASES_RAW_BYTES[table].fetch_add(bytes, Ordering::Relaxed);
}

pub(crate) fn add_supremacy_copy_ns(table: usize, ns: u64) {
    SUPREMACY_COPY_NS[table].fetch_add(ns, Ordering::Relaxed);
}

pub(crate) fn add_supremacy_raw_bytes(table: usize, bytes: u64) {
    SUPREMACY_RAW_BYTES[table].fetch_add(bytes, Ordering::Relaxed);
}

pub(crate) fn add_sunk_copy_ns(table: usize, ns: u64) {
    SUNK_COPY_NS[table].fetch_add(ns, Ordering::Relaxed);
}

pub(crate) fn add_sunk_raw_bytes(table: usize, bytes: u64) {
    SUNK_RAW_BYTES[table].fetch_add(bytes, Ordering::Relaxed);
}

pub(crate) fn add_radar_copy_ns(table: usize, ns: u64) {
    RADAR_COPY_NS[table].fetch_add(ns, Ordering::Relaxed);
}

pub(crate) fn add_radar_raw_bytes(table: usize, bytes: u64) {
    RADAR_RAW_BYTES[table].fetch_add(bytes, Ordering::Relaxed);
}

pub(crate) fn add_intel_copy_ns(table: usize, ns: u64) {
    INTEL_COPY_NS[table].fetch_add(ns, Ordering::Relaxed);
}

pub(crate) fn add_intel_raw_bytes(table: usize, bytes: u64) {
    INTEL_RAW_BYTES[table].fetch_add(bytes, Ordering::Relaxed);
}

pub(crate) fn add_weather_copy_ns(table: usize, ns: u64) {
    WEATHER_COPY_NS[table].fetch_add(ns, Ordering::Relaxed);
}

pub(crate) fn add_weather_raw_bytes(table: usize, bytes: u64) {
    WEATHER_RAW_BYTES[table].fetch_add(bytes, Ordering::Relaxed);
}

pub(crate) fn add_capitulation_copy_ns(table: usize, ns: u64) {
    CAPITULATION_COPY_NS[table].fetch_add(ns, Ordering::Relaxed);
}

pub(crate) fn add_capitulation_raw_bytes(table: usize, bytes: u64) {
    CAPITULATION_RAW_BYTES[table].fetch_add(bytes, Ordering::Relaxed);
}
