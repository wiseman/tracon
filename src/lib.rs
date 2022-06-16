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

pub fn for_each_adsbx_json<OP>(
    paths: &[String],
    skip_json_errors: bool,
    mut op: OP,
) -> Result<(), Error>
where
    OP: FnMut(adsbx_json::v2::Response) + Sync + Send,
{
    let bar = ProgressBar::new(paths.len().try_into().unwrap());
    bar.set_style(
        ProgressStyle::default_bar().template("{wide_bar} {pos}/{len} {eta} {elapsed_precise}"),
    );
    pariter::scope(|scope| {
        paths
            .iter()
            .parallel_map_scoped(scope, |path| match load_adsbx_json_file(path) {
                Ok(response) => Ok(response),
                Err(err) => Err((path, err)),
            })
            .for_each(|result| {
                match result {
                    Ok(response) => op(response),
                    Err((path, err)) => {
                        eprintln!("Error reading file {}: {}\n", path, err);
                        if !skip_json_errors {
                            // It's not ideal to just exit here, but
                            // pariter::scope doesn't seem to make it easy to
                            // propagate an error up.
                            std::process::exit(1);
                        }
                    }
                }
                bar.inc(1);
            });
    })
    .map_err(|e| Error::ParallelMapError(format!("{:?}", e)))
}

/// Turns an altitude into a number (where ground is 0).

pub fn alt_number(alt: AltitudeOrGround) -> i32 {
    match alt {
        AltitudeOrGround::OnGround => 0,
        AltitudeOrGround::Altitude(alt) => alt,
    }
}

/// The speed threshold to be considered a "fast mover".
pub const FAST_MOVER_SPEED_KTS: f64 = 350.0;

/// The length of time a fast mover must travel below FAST_MOVER_SPEED_KTS to
/// lost "fast mover" status.
pub const FAST_MOVER_TIMEOUT_MINS: i64 = 3;

/// A fast mover is an aircraft that has been moving at high speed.
#[derive(Debug)]
pub struct FastMover {
    pub hex: String,
    pub coords: [f64; 2],
    pub max_speed: f64,
    pub cur_speed: f64,
    pub cur_alt: i32,
    pub is_on_ground: bool,
    /// The last time the aircraft was seen moving faster than
    /// FAST_MOVER_SPEED_KTS.
    pub time_seen_fast: DateTime<Utc>,
    /// The number of consecutive updates where the aircraft was moving faster
    /// than FAST_MOVER_SPEED_KTS.
    pub fast_count: u32,
    pub seen: DateTime<Utc>,
}

impl FastMover {
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
        Ok(FastMover {
            hex: aircraft.hex.clone(),
            coords: [lon, lat],
            max_speed: spd,
            cur_speed: spd,
            cur_alt: alt,
            is_on_ground: aircraft_is_on_ground(aircraft),
            time_seen_fast: now - Duration::from_std(seen_pos).unwrap(),
            fast_count: 1,
            seen: now - Duration::from_std(aircraft.seen_pos.unwrap()).unwrap(),
        })
    }

    pub fn update(&mut self, now: DateTime<Utc>, aircraft: &Aircraft) {
        if let Some(spd) = aircraft.ground_speed_knots {
            self.cur_speed = spd;
            if spd > self.max_speed {
                self.max_speed = spd;
            }
            if self.cur_speed > FAST_MOVER_SPEED_KTS {
                self.time_seen_fast = now;
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
        self.coords = [aircraft.lon.unwrap(), aircraft.lat.unwrap()];
    }
}

// Checks whether an aircraft seems to be on the ground (or very close to it).

pub fn aircraft_is_on_ground(aircraft: &Aircraft) -> bool {
    (aircraft.barometric_altitude.is_some()
        && aircraft.barometric_altitude.as_ref().unwrap() == &AltitudeOrGround::OnGround)
        || (aircraft.geometric_altitude.is_some() && aircraft.geometric_altitude.unwrap() < 500)
}

/// Represents a potential target (slower-moving) aircraft.

#[derive(Debug)]
pub struct Target {
    pub hex: String,
    pub cur_speed: f64,
    pub cur_alt: i32,
    pub is_on_ground: bool,
    pub seen: DateTime<Utc>,
}

impl Target {
    pub fn new(now: DateTime<Utc>, aircraft: &Aircraft) -> Self {
        Self {
            hex: aircraft.hex.clone(),
            cur_speed: aircraft.ground_speed_knots.unwrap_or(0.0),
            cur_alt: aircraft.geometric_altitude.unwrap_or(0),
            is_on_ground: aircraft_is_on_ground(aircraft),
            seen: now - Duration::from_std(aircraft.seen_pos.unwrap()).unwrap(),
        }
    }
}

/// This is the type that we put in the spatial index (r-tree) to find
/// slow-movers near fast-movers.

pub type TargetLocation = GeomWithData<[f64; 2], Target>;
