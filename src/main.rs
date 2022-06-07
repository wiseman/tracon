use std::{io::Read, str::FromStr};

use adsbx_json::v2::{Aircraft, AltitudeOrGround};
use anyhow::{Context, Result};
use chrono::{prelude::*, Duration};
use geo::{point, prelude::*};
use indicatif::{ProgressBar, ProgressStyle};
use pariter::{scope, IteratorExt as _};
use rstar::primitives::GeomWithData;
use rstar::RTree;
use structopt::StructOpt;

#[derive(StructOpt, Debug)]
struct CliArgs {
    #[structopt(help = "Input files")]
    pub paths: Vec<String>,
    #[structopt(long, help = "Skip JSON decoding errors")]
    pub skip_json_errors: bool,
}

/// Loads a JSON file containing an ADS-B Exchange API response and parses it
/// into a struct.

pub fn load_adsbx_json(path: &str) -> Result<adsbx_json::v2::Response> {
    let mut json_contents = String::new();
    if path.ends_with(".bz2") {
        let file = std::fs::File::open(path)?;
        let mut decompressor = bzip2::read::MultiBzDecoder::new(file);
        decompressor.read_to_string(&mut json_contents)?;
    } else {
        std::fs::File::open(path)?.read_to_string(&mut json_contents)?;
    }
    adsbx_json::v2::Response::from_str(&json_contents).with_context(|| format!("Parsing {}", path))
}

// Processes a collection of files containing ADS-B Exchange API responses.
// Decompresses and parses files in parallel, but calls the callback function
// serially.

fn for_each_adsbx_json<OP>(args: &CliArgs, mut op: OP) -> Result<()>
where
    OP: FnMut(adsbx_json::v2::Response) + Sync + Send,
{
    let bar = ProgressBar::new(args.paths.len().try_into().unwrap());
    bar.set_style(
        ProgressStyle::default_bar().template("{wide_bar} {pos}/{len} {eta} {elapsed_precise}"),
    );
    scope(|scope| {
        args.paths
            .iter()
            .parallel_map_scoped(scope, |path| match load_adsbx_json(path) {
                Ok(response) => Ok(response),
                Err(err) => Err((path, err)),
            })
            .for_each(|result| {
                match result {
                    Ok(response) => op(response),
                    Err((path, err)) => {
                        if args.skip_json_errors {
                            eprintln!("Error reading file {}: {}\n", path, err);
                        } else {
                            eprintln!("Error reading file {}: {}\n", path, err);
                            std::process::exit(1);
                        }
                    }
                }
                bar.inc(1);
            });
    })
    .unwrap();
    bar.finish();
    Ok(())
}

pub fn alt_number(alt: AltitudeOrGround) -> i32 {
    match alt {
        AltitudeOrGround::OnGround => 0,
        AltitudeOrGround::Altitude(alt) => alt,
    }
}

const FAST_MOVER_SPEED_KTS: f64 = 350.0;
const FAST_MOVER_TIMEOUT_MINS: i64 = 3;

#[derive(Debug)]
struct FastMover {
    hex: String,
    coords: [f64; 2],
    max_speed: f64,
    cur_speed: f64,
    cur_alt: i32,
    is_on_ground: bool,
    time_seen_fast: DateTime<Utc>,
    fast_count: u32,
    seen: DateTime<Utc>,
}

impl FastMover {
    fn new(now: DateTime<Utc>, aircraft: &Aircraft) -> Self {
        FastMover {
            hex: aircraft.hex.clone(),
            coords: [aircraft.lon.unwrap(), aircraft.lat.unwrap()],
            max_speed: aircraft.ground_speed_knots.unwrap(),
            cur_speed: aircraft.ground_speed_knots.unwrap(),
            cur_alt: aircraft.geometric_altitude.unwrap(),
            is_on_ground: aircraft_is_on_ground(aircraft),
            time_seen_fast: now - Duration::from_std(aircraft.seen_pos.unwrap()).unwrap(),
            fast_count: 1,
            seen: now - Duration::from_std(aircraft.seen_pos.unwrap()).unwrap(),
        }
    }
}

fn aircraft_is_on_ground(aircraft: &Aircraft) -> bool {
    (aircraft.barometric_altitude.is_some()
        && aircraft.barometric_altitude.as_ref().unwrap() == &AltitudeOrGround::OnGround)
        || (aircraft.geometric_altitude.is_some() && aircraft.geometric_altitude.unwrap() < 500)
}

impl FastMover {
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

#[derive(Debug)]
struct Target {
    hex: String,
    cur_speed: f64,
    cur_alt: i32,
    is_on_ground: bool,
    seen: DateTime<Utc>,
}

impl Target {
    fn new(now: DateTime<Utc>, aircraft: &Aircraft) -> Self {
        Self {
            hex: aircraft.hex.clone(),
            cur_speed: aircraft.ground_speed_knots.unwrap_or(0.0),
            cur_alt: aircraft.geometric_altitude.unwrap_or(0),
            is_on_ground: aircraft_is_on_ground(aircraft),
            seen: now - Duration::from_std(aircraft.seen_pos.unwrap()).unwrap(),
        }
    }
}

type TargetLocation = GeomWithData<[f64; 2], Target>;

#[derive(Debug, Default)]
struct State {
    fast_movers: Vec<FastMover>,
    index: RTree<TargetLocation>,
}

fn process_adsbx_response(state: &mut State, response: adsbx_json::v2::Response) {
    let mut slow_movers = vec![];
    let now = response.now;
    for aircraft in &response.aircraft {
        if let (Some(lat), Some(lon), Some(gnd_speed), Some(_)) = (
            aircraft.lat,
            aircraft.lon,
            aircraft.ground_speed_knots,
            aircraft.geometric_altitude,
        ) {
            let hex = &aircraft.hex;
            match state.fast_movers.iter().position(|m| &m.hex == hex) {
                None => {
                    if gnd_speed > FAST_MOVER_SPEED_KTS {
                        state.fast_movers.push(FastMover::new(now, aircraft));
                    } else if gnd_speed < 250.0 {
                        slow_movers
                            .push(TargetLocation::new([lon, lat], Target::new(now, aircraft)));
                    }
                }
                Some(pos) => {
                    let m = &mut state.fast_movers[pos];
                    m.update(now, aircraft);
                }
            }
        }
    }
    // Now remove stale fast movers.
    state.fast_movers.retain(|m| {
        m.time_seen_fast > now - Duration::minutes(FAST_MOVER_TIMEOUT_MINS)
            && m.seen > now - Duration::minutes(FAST_MOVER_TIMEOUT_MINS)
    });

    if !state.fast_movers.is_empty() {
        const MAX_DIST_NM: f64 = 1.0;
        let max_dist_deg_2 = (MAX_DIST_NM / 60.0).powi(2);
        state.index = RTree::bulk_load(slow_movers);

        // Now look for non-fast movers that are close to known fast movers.
        // let mut interceptions = vec![];
        for fast_mover in &state.fast_movers {
            if !fast_mover.is_on_ground
                && fast_mover.fast_count >= 10
                && ((now - fast_mover.seen) < Duration::minutes(1))
            {
                let targets = state
                    .index
                    .locate_within_distance(fast_mover.coords, max_dist_deg_2);
                for target in targets {
                    let target_pt = point!(x: target.geom()[0], y: target.geom()[1]);
                    let fast_mover_pt = point!(x: fast_mover.coords[0], y: fast_mover.coords[1]);
                    let dist = target_pt.haversine_distance(&fast_mover_pt);
                    if target.data.hex != fast_mover.hex
                        && dist < 500.0
                        && !target.data.is_on_ground
                        && (target.data.cur_speed - fast_mover.cur_speed).abs() < 250.0
                        && (target.data.cur_alt - fast_mover.cur_alt).abs() < 500
                        && ((now - target.data.seen) < Duration::minutes(1))
                    {
                        println!(
                            "{} might have intercepted {} at {} ({:.1} m) {:?} {:?}: {}",
                            fast_mover.hex,
                            target.data.hex,
                            now,
                            dist,
                            fast_mover.coords,
                            target.geom(),
                            url(fast_mover, &target.data, now)
                        );
                    }
                }
            }
        }
    }
}

fn url(fast_mover: &FastMover, target: &Target, now: DateTime<Utc>) -> String {
    let mut url = String::new();
    url.push_str("https://globe.adsbexchange.com/?icao=");
    url.push_str(&fast_mover.hex);
    url.push(',');
    url.push_str(&target.hex);
    url.push_str("&showTrace=");
    url.push_str(&now.format("%Y-%m-%d").to_string());
    url.push_str(format!("&lat={}&lon={}", fast_mover.coords[1], fast_mover.coords[0]).as_str());
    url.push_str("&zoom=11");
    let start_time = now - Duration::minutes(5);
    let end_time = now + Duration::minutes(1);
    url.push_str(format!("&startTime={}", start_time.format("%H:%M")).as_str());
    url.push_str(format!("&endTime={}", end_time.format("%H:%M")).as_str());
    url
}

fn main() -> Result<(), String> {
    let args = CliArgs::from_args();
    eprintln!("Processing {} files", args.paths.len());
    let mut state = State::default();
    for_each_adsbx_json(&args, |response| {
        process_adsbx_response(&mut state, response)
    })
    .unwrap();
    Ok(())
}
