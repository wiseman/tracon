use std::{cmp::max, collections::HashMap};

use anyhow::Result;
use chrono::{prelude::*, Duration};
use geo::{point, prelude::*};
use interceptiondetector::{for_each_adsbx_json, Ac, Class, Interception, TargetLocation};
use rstar::{primitives::GeomWithData, RTree};
use structopt::StructOpt;

#[derive(StructOpt, Debug)]
struct CliArgs {
    #[structopt(help = "Input files")]
    pub paths: Vec<String>,
    #[structopt(long, help = "Skip JSON decoding errors")]
    pub skip_json_errors: bool,
}

/// This is the state that is kept across ADS-B Exchange API responses.

#[derive(Debug, Default)]
struct State {
    aircraft: HashMap<String, Ac>,
    num_ac_indexed: usize,
    num_ac_processed: usize,
    interceptions: Vec<Interception>,
}

fn process_adsbx_response(state: &mut State, response: adsbx_json::v2::Response) {
    let now = response.now;
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

    if !fast_movers.is_empty() {
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

        // Now look for non-fast movers that are close to known fast movers.
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
    }
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

fn url(fast_mover: &Ac, target: &Ac, now: DateTime<Utc>) -> String {
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
    // ADSBX startTime and endTime params only have 1 minute resolution, so
    // let's add 1 minute to make sure we actually cover the time of
    // interception.
    let end_time = now + Duration::minutes(0);
    url.push_str(format!("&startTime={}", start_time.format("%H:%M")).as_str());
    url.push_str(format!("&endTime={}", end_time.format("%H:%M")).as_str());
    url
}

fn main() -> Result<(), String> {
    let args = CliArgs::from_args();
    eprintln!("Processing {} files", args.paths.len());
    let mut state = State::default();
    for_each_adsbx_json(&args.paths, args.skip_json_errors, |response| {
        process_adsbx_response(&mut state, response)
    })
    .unwrap();
    eprintln!(
        "Indexed {} aircraft, processed {} aircraft, found {} interceptions",
        state.num_ac_indexed,
        state.num_ac_processed,
        state.interceptions.len()
    );
    for interception in state.interceptions {
        println!("{} {} intercepted {} at {} with {:.0} ft lateral separation, {} ft vertical separation",
        url(&interception.interceptor, &interception.target, interception.time),
        interception.interceptor.hex,
             interception.target.hex,
             interception.time,
             interception.lateral_separation_ft.round(),
             interception.vertical_separation_ft,
        );
    }
    Ok(())
}
