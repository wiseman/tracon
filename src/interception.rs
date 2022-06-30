use std::cmp::max;

use adsbx_json::v2::Aircraft;
use chrono::{prelude::*, Duration};
use geo::{point, HaversineDistance};
use indicatif::ProgressBar;
use rstar::{primitives::GeomWithData, RTree};
use std::collections::HashMap;

use crate::{aircraft_is_on_ground, alt_number, error::Error};

/// The speed threshold to be considered an interceptor.
pub const INTERCEPTOR_MIN_SPEED_KTS: f64 = 400.0;

/// The maximum speed of a potential target.
pub const TARGET_MAX_SPEED_KTS: f64 = 350.0;

/// The minimum speed of a potential target.
pub const TARGET_MIN_SPEED_KTS: f64 = 80.0;

/// The length of time an interceptor must travel below INTERCEPTOR_SPEED_KTS to
/// lose interceptor status.
pub const INTERCEPTOR_TIMEOUT_MINS: i64 = 3;

/// The different classifications of aircraft.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Class {
    /// Possible interceptor.
    Interceptor,
    /// Possible target.
    Target,
    /// Neither interceptor nor target.
    Other,
}

/// State we keep track of for each aircraft.
#[derive(Debug, Clone)]
pub struct Ac {
    pub hex: String,
    pub coords: Vec<(DateTime<Utc>, [f64; 2])>,
    pub max_speed: f64,
    pub cur_speed: f64,
    pub cur_alt: i32,
    pub is_on_ground: bool,
    /// The last time the aircraft was seen moving faster than
    /// INTERCEPTOR_MIN_SPEED_KTS.
    pub time_seen_fast: Option<DateTime<Utc>>,
    /// The number of updates where the aircraft was moving faster than
    /// INTERCEPTOR_SPEED_KTS.
    pub fast_count: u32,
    /// When was the aircraft last seen.
    pub seen: DateTime<Utc>,
}

impl Ac {
    pub fn new(now: DateTime<Utc>, aircraft: &Aircraft) -> Result<Self, Error> {
        let (lon, lat) = match (aircraft.lon, aircraft.lat) {
            (Some(lon), Some(lat)) => (lon, lat),
            _ => {
                return Err(Error::AircraftMissingData(format!(
                    "Aircraft {} is missing position data",
                    aircraft.hex
                )))
            }
        };
        let spd = match aircraft.ground_speed_knots {
            Some(spd) => spd,
            _ => {
                return Err(Error::AircraftMissingData(format!(
                    "Aircraft {} is missing ground speed data",
                    aircraft.hex
                )))
            }
        };
        let alt = match aircraft.geometric_altitude {
            Some(alt) => alt,
            _ => {
                return Err(Error::AircraftMissingData(format!(
                    "Aircraft {} is missing geometric altitude",
                    aircraft.hex
                )))
            }
        };
        let seen_pos = match aircraft.seen_pos {
            Some(seen_pos) => seen_pos,
            _ => {
                return Err(Error::AircraftMissingData(format!(
                    "Aircraft {} is missing seen_pos",
                    aircraft.hex
                )))
            }
        };
        let is_fast = spd > INTERCEPTOR_MIN_SPEED_KTS;
        Ok(Ac {
            hex: aircraft.hex.clone(),
            coords: vec![(now, [lon, lat])],
            max_speed: spd,
            cur_speed: spd,
            cur_alt: alt,
            is_on_ground: aircraft_is_on_ground(aircraft),
            time_seen_fast: if is_fast {
                Some(now - Duration::from_std(seen_pos).unwrap())
            } else {
                None
            },
            fast_count: if is_fast { 1 } else { 0 },
            seen: now - Duration::from_std(aircraft.seen_pos.unwrap()).unwrap(),
        })
    }

    // Updates aircraft state based on latest API response for that aircraft.
    pub fn update(&mut self, now: DateTime<Utc>, aircraft: &Aircraft) {
        if let Some(spd) = aircraft.ground_speed_knots {
            self.cur_speed = spd;
            self.max_speed = self.max_speed.max(spd);
            if self.cur_speed > INTERCEPTOR_MIN_SPEED_KTS {
                self.time_seen_fast = Some(now);
                self.fast_count += 1;
            }
        }
        self.cur_alt = aircraft.geometric_altitude.unwrap_or_else(|| {
            aircraft
                .barometric_altitude
                .clone()
                .map(alt_number)
                .unwrap_or(0)
        });
        self.is_on_ground = aircraft_is_on_ground(aircraft);
        self.seen = now - Duration::from_std(aircraft.seen_pos.unwrap()).unwrap();
        self.coords
            .push((now, [aircraft.lon.unwrap(), aircraft.lat.unwrap()]));
        // Keep the last 40 positions (about 10 minutes worth).
        if self.coords.len() > 40 {
            self.coords.remove(0);
        }
    }

    /// Returns the aircraft's most recent coordinates.
    pub fn cur_coords(&self) -> &(DateTime<Utc>, [f64; 2]) {
        self.coords.last().unwrap()
    }

    /// Returns the aircraft's oldest coordinates (usually from about 10 minutes
    /// ago).
    pub fn oldest_coords(&self) -> &(DateTime<Utc>, [f64; 2]) {
        self.coords.first().unwrap()
    }

    pub fn class(&self, now: DateTime<Utc>) -> Class {
        if let Some(time_seen_fast) = self.time_seen_fast {
            let elapsed = now.signed_duration_since(time_seen_fast);
            if elapsed.num_minutes() < INTERCEPTOR_TIMEOUT_MINS
                && self.fast_count > 10
                && !self.is_on_ground
            {
                return Class::Interceptor;
            }
        }
        if self.cur_speed > TARGET_MIN_SPEED_KTS
            && self.cur_speed < TARGET_MAX_SPEED_KTS
            && !self.is_on_ground
        {
            return Class::Target;
        }
        Class::Other
    }
}

/// This is the type that we put in the spatial index (r-tree) to find
/// slow-movers near fast-movers.

pub type TargetLocation = GeomWithData<[f64; 2], Ac>;

#[derive(Debug)]
pub struct Interception {
    pub interceptor: Ac,
    pub target: Ac,
    pub time: DateTime<Utc>,
    pub lateral_separation_ft: f64,
    pub vertical_separation_ft: i32,
}

/// This is the state that is kept across ADS-B Exchange API responses.

#[derive(Debug, Default)]
pub struct State {
    pub aircraft: HashMap<String, Ac>,
    pub num_ac_indexed: usize,
    pub num_ac_processed: usize,
    pub interceptions: Vec<Interception>,
}

pub fn process_adsbx_response(
    state: &mut State,
    response: adsbx_json::v2::Response,
    bar: &ProgressBar,
) -> Result<(), Error> {
    let now = response.now;

    // First classify each aircraft as a fast mover/interceptor, a slow
    // mover/target, or neither (which we don't care about).
    let mut fast_movers = vec![];
    let mut potential_tois: Vec<GeomWithData<[f64; 2], Ac>> = vec![];
    for aircraft in &response.aircraft {
        if let (Some(_), Some(_), Some(_), Some(_)) = (
            aircraft.lat,
            aircraft.lon,
            aircraft.ground_speed_knots,
            aircraft.geometric_altitude,
        ) {
            let hex = &aircraft.hex;
            // Insert or update the aircraft into the state.
            if let Some(ac) = state.aircraft.get_mut(&aircraft.hex) {
                ac.update(now, aircraft);
            } else {
                state
                    .aircraft
                    .insert(aircraft.hex.clone(), Ac::new(now, aircraft).unwrap());
            }
            let ac = state.aircraft.get(hex).unwrap();
            match ac.class(now) {
                Class::Interceptor => {
                    fast_movers.push(ac.clone());
                }
                Class::Target => {
                    potential_tois.push(TargetLocation::new(ac.cur_coords().1, ac.clone()));
                }
                _ => {}
            }
        }
    }
    // Now remove stale aircraft.
    state
        .aircraft
        .retain(|_, ac| (now - ac.seen) < Duration::minutes(10));

    if fast_movers.is_empty() {
        return Ok(());
    }
    // The r-tree treats coordinates as cartesian, but they're geospatial
    // (spherical). So we use the fact that one degree (of latitude, anyway)
    // is 60 nautical miles and use the r-tree index to look up any
    // potential targets kinda-close to each fast-mover, then do a more
    // precise filtering using Haversine distance.  Of course one degree in
    // longitude/the X-axis represents a variable distance depending on
    // where it is on Earth, but we're not usually looking at planes flying
    // over a pole.
    //
    // An alternative might be to use H3?
    state.num_ac_indexed += &potential_tois.len();
    let spatial_index = RTree::bulk_load(potential_tois);
    const MAX_DIST_NM: f64 = 0.5;
    let max_dist_deg_2 = (MAX_DIST_NM / 60.0).powi(2);

    // For each fast mover, find any potential targets that are close enough.
    for fast_mover in fast_movers {
        let fast_mover_coords = fast_mover.cur_coords().1;
        let targets = spatial_index.locate_within_distance(fast_mover_coords, max_dist_deg_2);
        for target in targets {
            let target_coords = target.data.cur_coords().1;
            state.num_ac_processed += 1;
            let target_pt = point!(x: target_coords[0], y: target_coords[1]);
            let fast_mover_pt = point!(x: fast_mover_coords[0], y: fast_mover_coords[1]);
            let dist = target_pt.haversine_distance(&fast_mover_pt);
            let alt_diff = (target.data.cur_alt - fast_mover.cur_alt).abs();
            if dist < 500.0
                && (target.data.cur_speed - fast_mover.cur_speed).abs() < 150.0
                && alt_diff < 500
                && ((now - target.data.seen) < Duration::minutes(1))
                && started_far_apart(&fast_mover, &target.data)
            {
                // Consider this a duplicate interception if the same
                // fast_mover intercepted the same target within the
                // past 10 minutes.
                if state.interceptions.iter().any(|i| {
                    i.interceptor.hex == fast_mover.hex
                        && i.target.hex == target.data.hex
                        && i.time > now - Duration::minutes(10)
                }) {
                    continue;
                }

                let interception = Interception {
                    interceptor: fast_mover.clone(),
                    target: target.data.clone(),
                    lateral_separation_ft: dist * 3.28084,
                    vertical_separation_ft: alt_diff,
                    time: now,
                };
                state.interceptions.push(interception);
                eprintln!(
                    "\n{} might have intercepted {} at {}",
                    fast_mover.hex, target.data.hex, now,
                );
            }
        }
    }

    bar.set_message(format!(
        "[ {} interceptions found ]",
        state.interceptions.len()
    ));
    Ok(())
}

// Function that checks whether the two aircraft were more than 10 miles apart
// in the past.
//
// First we look at the oldest position for each aircraft, then find the most
// recent timestamp on those positions to be our time of comparison. Then we
// find the timestamped position for the other aircraft that is closest in time
// to the time of comparison.
fn started_far_apart(fast_mover: &Ac, target: &Ac) -> bool {
    let oldest_fm_ts = fast_mover.oldest_coords().0;
    let oldest_t_ts = target.coords[0].0;
    let comparison_ts = max(oldest_fm_ts, oldest_t_ts);
    let mut temp_fast_mover_coords = fast_mover.coords.clone();
    let mut temp_target_coords = target.coords.clone();
    temp_fast_mover_coords.sort_by_key(|c| (c.0 - comparison_ts).num_seconds().abs());
    temp_target_coords.sort_by_key(|c| (c.0 - comparison_ts).num_seconds().abs());
    let dist = point!(x: temp_fast_mover_coords[0].1[0], y: temp_fast_mover_coords[0].1[1])
        .haversine_distance(&point!(x: temp_target_coords[0].1[0], y: temp_target_coords[0].1[1]));
    dist > 10.0 * 1609.34
}

/// Generates an ADS-B Exchange URL for an interception.

pub fn url(fast_mover: &Ac, target: &Ac, now: DateTime<Utc>) -> String {
    let mut url = String::new();
    url.push_str("https://globe.adsbexchange.com/?icao=");
    url.push_str(&fast_mover.hex);
    url.push(',');
    url.push_str(&target.hex);
    url.push_str("&showTrace=");
    url.push_str(&now.format("%Y-%m-%d").to_string());
    let fast_mover_coords = fast_mover.coords.iter().last().unwrap().1;
    url.push_str(format!("&lat={}&lon={}", fast_mover_coords[1], fast_mover_coords[0]).as_str());
    url.push_str("&zoom=11");
    let start_time = now - Duration::minutes(5);
    let end_time = now + Duration::minutes(1);
    url.push_str(format!("&startTime={}", start_time.format("%H:%M")).as_str());
    url.push_str(format!("&endTime={}", end_time.format("%H:%M:%S")).as_str());
    url
}
