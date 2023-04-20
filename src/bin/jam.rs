use std::collections::{HashMap, HashSet};

use chrono::prelude::*;
use dump::{for_each_adsbx_json, in_bbox, Bounds};
use structopt::StructOpt;

#[derive(StructOpt, Debug)]
struct CliArgs {
    // The interval (in seconds) to group aircraft by.
    #[structopt(help = "Interval (seconds)")]
    pub interval: u64,
    // The paths to the ADS-B Exchange JSON files.
    #[structopt(help = "Input files")]
    pub paths: Vec<String>,
    #[structopt(
        long,
        value_name = "min_lat,min_lon,max_lat,max_lon",
        allow_hyphen_values = true,
        help = "Filter by bounding box: min_lat,min_lon,max_lat,max_lon"
    )]
    pub bbox: Option<Bounds>,
}

// Keys consist of the following:
// Date, hour of day, country.
#[derive(Hash, Eq, PartialEq, PartialOrd, Ord, Debug)]
struct Key {
    datetime: String,
}

fn main() -> Result<(), String> {
    let args = CliArgs::from_args();
    let mut data = HashMap::<Key, HashSet<u32>>::new();

    for_each_adsbx_json(&args.paths, |adsbx_data| {
        // Compute the datetime key based on the specified interval. Examples:
        //
        // Interval 10 seconds:
        // 2020-01-01 00:00:07.123 -> 2020-01-01 00:00:00 UTC
        // 2020-01-01 00:00:17.123 -> 2020-01-01 00:00:10 UTC
        // Interval 1 minute:
        // 2020-01-01 00:00:07.123 -> 2020-01-01 00:00:00 UTC
        // 2020-01-01 00:01:17.123 -> 2020-01-01 00:01:00 UTC
        let datetime = adsbx_data
            .now
            .with_nanosecond(0)
            .unwrap()
            .checked_sub_signed(chrono::Duration::seconds(
                adsbx_data.now.second() as i64 % args.interval as i64,
            ))
            .unwrap()
            .to_rfc3339_opts(chrono::SecondsFormat::Secs, true);
        adsbx_data
            .aircraft
            .iter()
            // If a bounding box was specified, only process aircraft within it.
            .filter(|a| in_bbox(&args.bbox, a))
            .for_each(|ac| {
                // If the aircraft has bad gps, add it to the hashset for this key.
                if ac.gps_ok_before.is_some() {
                    let key = Key {
                        datetime: datetime.clone(),
                    };
                    // Parse the hex into a u32.
                    let hex = u32::from_str_radix(&ac.hex, 16).unwrap();
                    data.entry(key).or_insert_with(HashSet::new).insert(hex);
                }
            });
        None
    });
    // Write data out as CSV, with sorted keys.
    let mut keys = data.keys().collect::<Vec<_>>();
    keys.sort();
    for key in keys {
        println!("{},{}", key.datetime, data[key].len());
    }
    Ok(())
}
