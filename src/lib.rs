use std::{io::Read, str::FromStr};

use adsbx_json::v2::Aircraft;
use anyhow::{Context, Result as AnyResult};
use indicatif::{ProgressBar, ProgressStyle};
use pariter::IteratorExt;

pub mod db;

/// Loads a JSON file containing an ADS-B Exchange API response and parses it
/// into a struct.
pub fn load_adsbx_json(path: &str) -> AnyResult<adsbx_json::v2::Response> {
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

pub fn for_each_adsbx_json<OP>(paths: &[String], mut op: OP)
where
    OP: FnMut(adsbx_json::v2::Response) -> Option<String> + Sync + Send,
{
    let bar = ProgressBar::new(paths.len().try_into().unwrap());
    bar.set_style(
        ProgressStyle::default_bar()
            .template("{wide_bar} {pos}/{len} {eta} {elapsed_precise} | {msg}"),
    );
    pariter::scope(|scope| {
        paths
            .iter()
            .cloned()
            .parallel_map_scoped(scope, |path| match load_adsbx_json(&path) {
                Ok(data) => {
                    bar.inc(1);
                    Ok((path, data))
                }
                Err(e) => {
                    eprintln!("Error loading {}: {}", path, e);
                    Err(e)
                }
            })
            .for_each(|result| match result {
                Ok((_, data)) => {
                    if let Some(msg) = op(data) {
                        bar.set_message(msg);
                    }
                }
                Err(_e) => {}
            });
    })
    .unwrap();
    bar.finish();
}

/// Represents a bounding box. Used for filtering data to a region of interest.
#[derive(Debug, Clone, Copy)]
pub struct Bounds {
    pub min_lat: f64,
    pub min_lon: f64,
    pub max_lat: f64,
    pub max_lon: f64,
}

impl FromStr for Bounds {
    type Err = anyhow::Error;

    fn from_str(s: &str) -> AnyResult<Self> {
        let mut parts = s.split(',');
        let min_lat = parts
            .next()
            .ok_or_else(|| anyhow::anyhow!("missing min lat"))?
            .parse()?;
        let min_lon = parts
            .next()
            .ok_or_else(|| anyhow::anyhow!("missing min lon"))?
            .parse()?;
        let max_lat = parts
            .next()
            .ok_or_else(|| anyhow::anyhow!("missing max lat"))?
            .parse()?;
        let max_lon = parts
            .next()
            .ok_or_else(|| anyhow::anyhow!("missing max lon"))?
            .parse()?;
        Ok(Bounds {
            min_lat,
            min_lon,
            max_lat,
            max_lon,
        })
    }
}

/// Returns true if the aircraft is in the bounding box, or there is no bounding box.
pub fn in_bbox(bbox: &Option<Bounds>, aircraft: &Aircraft) -> bool {
    match bbox {
        None => true,
        Some(bbox) => match (aircraft.lat, aircraft.lon) {
            (None, None) => false,
            (None, _) => false,
            (_, None) => false,
            (Some(lat), Some(lon)) => {
                lat >= bbox.min_lat
                    && lat <= bbox.max_lat
                    && lon >= bbox.min_lon
                    && lon <= bbox.max_lon
            }
        },
    }
}
