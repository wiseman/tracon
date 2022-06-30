use std::{io::Read, str::FromStr};

use adsbx_json::v2::{Aircraft, AltitudeOrGround};
use error::Error;
use indicatif::{ProgressBar, ProgressStyle};
use pariter::IteratorExt;

pub mod error;
pub mod interception;

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

// Checks whether an aircraft seems to be on the ground (or very close to it).

pub fn aircraft_is_on_ground(aircraft: &Aircraft) -> bool {
    (aircraft.barometric_altitude.is_some()
        && aircraft.barometric_altitude.as_ref().unwrap() == &AltitudeOrGround::OnGround)
        || (aircraft.geometric_altitude.is_some() && aircraft.geometric_altitude.unwrap() < 500)
}
