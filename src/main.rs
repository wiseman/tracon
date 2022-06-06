use std::{io::Read, str::FromStr};

use adsbx_json::v2::{Aircraft, AltitudeOrGround};
use anyhow::{Context, Result};
use chrono::{prelude::*, Duration};
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

#[derive(Debug)]
struct FastMover {
    hex: String,
    coords: [f64; 2],
    max_speed: f64,
    cur_speed: f64,
    cur_alt: i32,
    time_seen_fast: DateTime<Utc>,
}

impl FastMover {
    pub fn update(&mut self, now: DateTime<Utc>, aircraft: &Aircraft) {
        if let Some(spd) = aircraft.ground_speed_knots {
            self.cur_speed = spd;
            if spd > self.max_speed {
                self.max_speed = spd;
            }
        }
        self.cur_alt = aircraft.geometric_altitude.unwrap_or(0);
        if self.cur_speed > 350.0 && self.cur_alt <= 10000 {
            self.time_seen_fast = now;
        }
    }
}

#[derive(Debug)]
struct Target {
    hex: String,
    cur_speed: f64,
    cur_alt: i32,
}

impl From<&Aircraft> for Target {
    fn from(aircraft: &Aircraft) -> Self {
        Self {
            hex: aircraft.hex.clone(),
            cur_speed: aircraft.ground_speed_knots.unwrap_or(0.0),
            cur_alt: aircraft.geometric_altitude.unwrap_or(0),
        }
    }
}

type TargetLocation = GeomWithData<[f64; 2], Target>;

#[derive(Debug, Default)]
struct State {
    fast_movers: Vec<FastMover>,
    index: RTree<TargetLocation>,
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

pub fn alt_number(alt: &AltitudeOrGround) -> i32 {
    match alt {
        AltitudeOrGround::OnGround => -1000,
        AltitudeOrGround::Altitude(alt) => *alt,
    }
}

fn process_adsbx_response(state: &mut State, response: adsbx_json::v2::Response) {
    let mut slow_movers = vec![];
    for aircraft in &response.aircraft {
        if let (Some(lat), Some(lon), Some(gnd_speed), Some(alt)) = (
            aircraft.lat,
            aircraft.lon,
            aircraft.ground_speed_knots,
            aircraft.geometric_altitude,
        ) {
            if gnd_speed > 350.0 && alt <= 10000 {
                let hex = aircraft.hex.clone();
                let time_seen_fast = response.now;
                match state.fast_movers.iter().position(|m| m.hex == hex) {
                    None => {
                        state.fast_movers.push(FastMover {
                            hex,
                            coords: [lon, lat],
                            max_speed: gnd_speed,
                            cur_speed: gnd_speed,
                            cur_alt: alt,
                            time_seen_fast,
                        });
                    }
                    Some(pos) => {
                        let m = &mut state.fast_movers[pos];
                        m.update(time_seen_fast, aircraft);
                    }
                }
            }
        } else if let (Some(lat), Some(lon)) = (aircraft.lat, aircraft.lon) {
            slow_movers.push(TargetLocation::new([lon, lat], aircraft.into()));
        }
    }
    // Now remove stale fast movers.
    let now = response.now;
    state
        .fast_movers
        .retain(|m| m.time_seen_fast > now - Duration::minutes(5));

    if !state.fast_movers.is_empty() {
        state.index = RTree::bulk_load(slow_movers);

        // Now look for non-fast movers that are close to known fast movers.
        // let mut interceptions = vec![];
        for fast_mover in &state.fast_movers {
            let targets = state.index.locate_within_distance(fast_mover.coords, 0.001);
            for target in targets {
                if target.data.hex != fast_mover.hex
                    && target.data.cur_alt > 1000
                    && fast_mover.cur_alt > 1000
                    && (target.data.cur_speed - fast_mover.cur_speed).abs() < 150.0
                    && (target.data.cur_alt - fast_mover.cur_alt).abs() < 2000
                {
                    println!(
                        "\n{} might have intercepted {} at {}: {}\n",
                        fast_mover.hex,
                        target.data.hex,
                        now,
                        url(fast_mover, &target.data, now)
                    );
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
    url
}

fn main() -> Result<(), String> {
    let args = CliArgs::from_args();
    println!("Processing {} files", args.paths.len());
    let mut state = State::default();
    for_each_adsbx_json(&args, |response| {
        process_adsbx_response(&mut state, response)
    })
    .unwrap();
    Ok(())
}
