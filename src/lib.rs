use std::{io::Read, str::FromStr};

use adsbx_json::v2::{Aircraft, AltitudeOrGround};
use chrono::{prelude::*, Duration};
use error::Error;
use indicatif::{ProgressBar, ProgressStyle};
use pariter::IteratorExt;
use rstar::primitives::GeomWithData;

pub mod error;

/// Loads a JSON file containing an ADS-B Exchange API response and parses it
/// into a struct.

pub fn load_adsbx_json_file(path: &str) -> Result<adsbx_json::v2::Response, Error> {
    let mut json_contents = String::new();
    if path.ends_with(".bz2") {
        let file = std::fs::File::open(path).map_err(|e| Error::JsonLoadError(e.to_string()))?;
        // Need to use MultiBZDecoder to decode something compressed with pbzip2.
        let mut decompressor = bzip2::read::MultiBzDecoder::new(file);
        decompressor
            .read_to_string(&mut json_contents)
            .map_err(|e| Error::JsonLoadError(e.to_string()))?;
    } else {
        std::fs::File::open(path)
            .map_err(|e| Error::JsonLoadError(e.to_string()))?
            .read_to_string(&mut json_contents)
            .map_err(|e| Error::JsonLoadError(e.to_string()))?;
    }
    adsbx_json::v2::Response::from_str(&json_contents)
        .map_err(|e| Error::JsonLoadError(e.to_string()))
}

// Processes a collection of files containing ADS-B Exchange API responses.
// Decompresses and parses files in parallel, but calls the callback function
// serially.

pub fn for_each_adsbx_json<F>(
    paths: &[String],
    skip_json_errors: bool,
    mut f: F,
) -> Result<(), Error>
where
    F: FnMut(adsbx_json::v2::Response, &ProgressBar) -> Result<(), Error>,
{
    let bar = ProgressBar::new(paths.len().try_into().unwrap());
    bar.set_style(
        ProgressStyle::default_bar()
            .template("{wide_bar} {pos}/{len} {eta} {elapsed_precise} {msg}"),
    );
    let r = pariter::scope(|scope| {
        paths
            .iter()
            .parallel_map_scoped(scope, |path| match load_adsbx_json_file(path) {
                Ok(response) => Ok(response),
                Err(err) => Err((path, err)),
            })
            .try_for_each(|result| {
                let r = match result {
                    Ok(response) => f(response, &bar),
                    Err((path, err)) => {
                        eprintln!("Error reading file {}: {}\n", path, err);
                        if !skip_json_errors {
                            Err(err)
                        } else {
                            Ok(())
                        }
                    }
                };
                bar.inc(1);
                r
            })
    })
    .map_err(|e| Error::ParallelMapError(format!("{:?}", e)));
    bar.finish();
    match r {
        Ok(_) => Ok(()),
        Err(e) => Err(e),
    }
}

pub fn try_fold_adsbx_json<B, F, R>(
    paths: &[String],
    skip_json_errors: bool,
    init: B,
    mut f: F,
) -> Result<B, Error>
where
    F: FnMut(B, adsbx_json::v2::Response, &ProgressBar) -> Result<B, Error>,
{
    let bar = ProgressBar::new(paths.len().try_into().unwrap());
    bar.set_style(
        ProgressStyle::default_bar()
            .template("{wide_bar} {pos}/{len} {eta} {elapsed_precise} {msg}"),
    );
    let r = pariter::scope(|scope| {
        paths
        .iter()
        .parallel_map_scoped(scope, |path| match load_adsbx_json_file(path) {
            Ok(response) => Ok(response),
            Err(err) => Err((path, err)),
        })
        .try_fold(init, |init, result| {
            let r = match result {
                Ok(response) => f(init, response, &bar),
                Err((path, err)) => {
                    eprintln!("Error reading file {}: {}\n", path, err);
                    if !skip_json_errors {
                        Err(Error::JsonLoadError("BLAH".to_string()))
                    } else {
                        Ok(init)
                    }
                }
            };
            bar.inc(1);
            r
        })
    })
    .map_err(|e| Error::ParallelMapError(format!("{:?}", e)));
    bar.finish();
    match r {
        Ok(b) => b,
        Err(e) => Err(e),
    }
}

/// Turns an altitude into a number (where ground is 0).

pub fn alt_number(alt: AltitudeOrGround) -> i32 {
    match alt {
        AltitudeOrGround::OnGround => 0,
        AltitudeOrGround::Altitude(alt) => alt,
    }
}

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

// Checks whether an aircraft seems to be on the ground (or very close to it).

pub fn aircraft_is_on_ground(aircraft: &Aircraft) -> bool {
    (aircraft.barometric_altitude.is_some()
        && aircraft.barometric_altitude.as_ref().unwrap() == &AltitudeOrGround::OnGround)
        || (aircraft.geometric_altitude.is_some() && aircraft.geometric_altitude.unwrap() < 500)
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
