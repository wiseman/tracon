use anyhow::Result;
use chrono::{prelude::*, Duration};
use geo::{point, prelude::*};
use interceptiondetector::{
    for_each_adsbx_json, FastMover, Target, TargetLocation, FAST_MOVER_SPEED_KTS,
    FAST_MOVER_TIMEOUT_MINS,
};
use rstar::RTree;
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
    fast_movers: Vec<FastMover>,
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
                        state
                            .fast_movers
                            .push(FastMover::new(now, aircraft).unwrap());
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
        // The r-tree treats coordinates as cartesian, but they're geospatial
        // (spherical). So we use the fact that one degree (of latitude, anyway)
        // is 60 nautical miles and use the r-tree index to look up any
        // potential targets kinda-close to each fast-mover, then do a more
        // precise filtering using Haversine distance.  Of course one degree in
        // the X-axis represents a variable distance depending on where it is on
        // Earth, but we're not usually looking at planes flying over a pole.
        //
        // An alternative might be to use H3?
        let spatial_index = RTree::bulk_load(slow_movers);
        const MAX_DIST_NM: f64 = 3.0;
        let max_dist_deg_2 = (MAX_DIST_NM / 60.0).powi(2);

        // Now look for non-fast movers that are close to known fast movers.
        for fast_mover in &state.fast_movers {
            if !fast_mover.is_on_ground
                && fast_mover.fast_count >= 10
                && ((now - fast_mover.seen) < Duration::minutes(1))
            {
                let targets =
                    spatial_index.locate_within_distance(fast_mover.coords, max_dist_deg_2);
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

/// Generates an ADS-B Exchange URL for an interception.

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
    // ADSBX startTime and endTime params only have 1 minute resolution, so
    // let's add 1 minute to make sure we actually cover the time of
    // interception.
    let end_time = now + Duration::minutes(1);
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
    Ok(())
}
