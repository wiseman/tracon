/// Detects aircrafts takeoffs from ADS-B data.
use chrono::{prelude::*, Duration};
use dump::for_each_adsbx_json;
use geo::algorithm::vincenty_distance::VincentyDistance;
use std::collections::HashMap;
use structopt::StructOpt;

#[derive(StructOpt, Debug)]
struct CliArgs {
    #[structopt(help = "Input files")]
    pub paths: Vec<String>,
}

/// Timestamped 2D coordinates with altitude.
#[derive(Debug)]
struct Pos {
    time: DateTime<Utc>,
    point: geo_types::Point<f64>,
}

/// What we keep track of for each aircraft.
#[derive(Default)]
struct AcState {
    recent_positions: Vec<Pos>,
}

/// Holds information about a duplicate hex use.
struct HexDupe {
    /// Time of the duplicate appearance.
    time: DateTime<Utc>,
    distance_miles: f64,
    time_delta: Duration,
}

#[derive(Default)]
struct AppState {
    aircraft: HashMap<String, AcState>,
    hex_dupes: HashMap<String, HexDupe>,
}

trait HexDuping {
    fn hex_dupe(&self) -> Option<HexDupe>;
}

impl HexDuping for AcState {
    fn hex_dupe(&self) -> Option<HexDupe> {
        // Get the last 2 positions and see if they're more than 500 miles apart.
        let positions = self
            .recent_positions
            .iter()
            .rev()
            .take(2)
            .collect::<Vec<_>>();
        if positions.len() < 2 {
            return None;
        }
        let pos1 = positions[0];
        let pos2 = positions[1];
        // Use geo to compute the distance between the two points.
        if let Ok(dist) = pos1.point.vincenty_distance(&pos2.point) {
            if dist > 500_000.0 * 1.61 && (pos1.time - pos2.time).num_minutes().abs() <= 15 {
                Some(HexDupe {
                    time: pos1.time,
                    distance_miles: dist / 1609.344,
                    time_delta: pos1.time - pos2.time,
                })
            } else {
                None
            }
        } else {
            None
        }
    }
}

// Unit tests
#[cfg(test)]
mod tests {
    // Note this useful idiom: importing names from outer (for mod tests) scope.
    use super::*;

    #[test]
    fn test_taking_off1() {
        env_logger::init();
        let mut ac_state = AcState {
            recent_positions: vec![
                Pos {
                    time: Utc::now(),
                    point: geo_types::Point::new(0.0, 0.0),
                },
                Pos {
                    time: Utc::now() + Duration::seconds(1),
                    point: geo_types::Point::new(1.0, 0.0),
                },
            ],
        };
        assert!(ac_state.hex_dupe().is_none());
        ac_state.recent_positions.push(Pos {
            time: Utc::now() + Duration::seconds(2),
            point: geo_types::Point::new(100.0, 0.0),
        });
        assert!(ac_state.hex_dupe().is_some());
    }
}

fn main() -> Result<(), String> {
    // Init the env_logger and write to stdout.
    env_logger::Builder::from_default_env()
        .target(env_logger::Target::Stdout)
        .init();
    let args = CliArgs::from_args();

    let mut state = AppState::default();
    println!("time,hex,distance_miles,time_delta,url");

    for_each_adsbx_json(&args.paths, |adsbx_data| {
        // let date = adsbx_data.now.format("%Y-%m-%d").to_string();
        // let hour = adsbx_data.now.hour();
        adsbx_data.aircraft.iter().for_each(|ac| {
            // Check for lat and lon.
            if let (Some(lat), Some(lon)) = (ac.lat, ac.lon) {
                let geo_point = geo_types::Point::new(lon, lat);
                    // println!("{} is inside polygon {},{}", ac.hex);
                    let ac_state = state
                        .aircraft
                        .entry(ac.hex.clone())
                        .or_insert_with(AcState::default);
                    ac_state.recent_positions.push(Pos {
                        time: adsbx_data.now,
                        point: geo_point,
                    });
                    // Keep only the last 30 minutes of positions for the aircraft.
                    ac_state.recent_positions.retain(|pos| {
                        adsbx_data.now - pos.time < Duration::minutes(30)
                    });
                    if let Some(dupe) = state
                        .aircraft
                        .entry(ac.hex.clone())
                        .or_insert_with(AcState::default)
                        .hex_dupe()
                    {
                        // Consider it a dupe if either it isn't in
                        // recent_takeoffs, or it is in recent_takeoffs but was
                        // added more than 30 minutes ago.
                        if let Some(prev_dupe) = state.hex_dupes.get(&ac.hex) {
                            if dupe.time - prev_dupe.time < Duration::minutes(30) {
                                return;
                            }
                        }
                        // Compute a start time that is 15 minutes before the dupe time. If that time is from the day before, clamp it to 00:00 of the same day.
                        let start_time = if dupe.time.hour() < 1 && dupe.time.minute() < 15 {
                            dupe.time.date().and_hms(0, 0, 0)
                        } else {
                            dupe.time - Duration::minutes(15)
                        };
                        // Compute an end time that is 15 minutes after the dupe time. If that time is from the day after, clamp it to 23:59 of the same day.
                        let end_time = if dupe.time.hour() > 23 && dupe.time.minute() > 45 {
                            dupe.time.date().and_hms(23, 59, 59)
                        } else {
                            dupe.time + Duration::minutes(15)
                        };
                        
                        // Create an adsbx url that looks like
                        // https://globe.adsbexchange.com/?icao=<hex>>&lat=<lat>>&lon=<lon>&zoom=14&showTrace=YYYY-MM-DD&trackLabels&startTime=HH:MM&endTime=HH:MM
                        let url = format!(
                            "https://globe.adsbexchange.com/?icao={}&showTrace={}&trackLabels&startTime={}&endTime={}",
                            ac.hex,
                            dupe.time.format("%Y-%m-%d"),
                            start_time.format("%H:%M"),
                            end_time.format("%H:%M"),
                        );
                        // Print miles with 0 decimal places.
                        println!(
                            "{},{},{:.0},{},{}",
                            dupe.time,
                            ac.hex,
                            dupe.distance_miles,
                            dupe.time_delta.num_seconds(),
                            url
                        );
                        state.hex_dupes.insert(ac.hex.clone(), dupe);
                    }
            }
        });
        if !state.hex_dupes.is_empty() {
            Some(format!("{} dupes found", state.hex_dupes.len(),))
        } else {
            None
        }
    });
    // println!("{} inside, {} outside", state.num_inside, state.num_outside);
    Ok(())
}
