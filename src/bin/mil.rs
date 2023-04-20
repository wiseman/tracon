use std::collections::{HashMap, HashSet};

use chrono::prelude::*;
use dump::for_each_adsbx_json;
use structopt::lazy_static::lazy_static;
use structopt::StructOpt;

#[derive(StructOpt, Debug)]
struct CliArgs {
    #[structopt(help = "Input files")]
    pub paths: Vec<String>,
}

// Keys consist of the following:
// Date, hour of day, country.
#[derive(Hash, Eq, PartialEq, PartialOrd, Ord, Debug)]
struct Key {
    date: String,
    hour: u32,
    h3_cell: h3ron::H3Cell,
    country: &'static str,
}

lazy_static! {
    static ref ALLOCS: aircraft_icao_range::Allocs = aircraft_icao_range::Allocs::new();
}

const H3_RES: u8 = 0;

fn main() -> Result<(), String> {
    let args = CliArgs::from_args();
    let mut data = HashMap::<Key, HashSet<u32>>::new();

    for_each_adsbx_json(&args.paths, |adsbx_data| {
        let date = adsbx_data.now.format("%Y-%m-%d").to_string();
        let hour = adsbx_data.now.hour();
        adsbx_data.aircraft.iter().for_each(|ac| {
            if !ac.database_flags.is_military() {
                return;
            }
            // Check for lat and lon.
            if let (Some(lat), Some(lon)) = (ac.lat, ac.lon) {
                // get h3 index from lat, lon.
                let h3_cell =
                    h3ron::H3Cell::from_coordinate(geo_types::Coord::from((lon, lat)), H3_RES)
                        .unwrap();
                // Check the cache for country first, then fall back to the
                // slower lookup.
                // Convert ac.hex from hex string to u32.
                let mode_s = u32::from_str_radix(&ac.hex, 16).unwrap();
                let country = ALLOCS.find(mode_s).unwrap_or("Unknown");
                let key = Key {
                    date: date.clone(),
                    hour,
                    h3_cell,
                    country,
                };
                let seen = data.entry(key).or_insert_with(HashSet::new);
                seen.insert(mode_s);
            }
        });
        None
    });
    // Write data out as CSV, with sorted keys.
    let mut keys = data.keys().collect::<Vec<_>>();
    keys.sort();
    for key in keys {
        println!(
            "{},{},{:x},{},{}",
            key.date,
            key.hour,
            h3ron::Index::h3index(&key.h3_cell),
            key.country,
            // Output the hexes as a comma separated list, sorted lexically.
            data[key]
                .iter()
                .map(|hex| format!("{:x}", hex))
                .collect::<Vec<_>>()
                .join("|"),
        );
    }
    Ok(())
}
