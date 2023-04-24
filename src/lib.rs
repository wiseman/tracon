use std::pin::Pin;
use std::sync::Arc;
use std::{io::Read, str::FromStr};

use adsbx_json::v2::Aircraft;
use anyhow::{Context, Result as AnyResult};
use futures::Future;
use indicatif::{ProgressBar, ProgressStyle};
use rayon::prelude::*;
use std::sync::Mutex;

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

pub fn for_each_adsbx_json<OP>(paths: &[String], op: OP)
where
    OP: FnMut(adsbx_json::v2::Response) -> Pin<Box<dyn Future<Output = Option<String>> + Send>>
        + Sync
        + Send
        + 'static,
{
    let bar = ProgressBar::new(paths.len().try_into().unwrap());
    bar.set_style(
        ProgressStyle::default_bar()
            .template("{wide_bar} {pos}/{len} {eta} {elapsed_precise} | {msg}"),
    );

    let pool = rayon::ThreadPoolBuilder::new()
        .num_threads(num_cpus::get())
        .build()
        .unwrap();

    let op = Arc::new(Mutex::new(op));

    pool.install(|| {
        paths
            .par_iter()
            .map(|path| {
                let result = load_adsbx_json(&path);
                bar.inc(1);
                (path, result)
            })
            .for_each_with(
                tokio::runtime::Handle::current(),
                |handle, (path, result)| match result {
                    Ok(data) => {
                        let mut op = op.lock().unwrap();
                        let fut = op(data);
                        drop(op);
                        let msg = handle.block_on(fut);
                        if let Some(msg) = msg {
                            bar.set_message(msg);
                        }
                    }
                    Err(e) => {
                        eprintln!("Error loading {}: {}", path, e);
                    }
                },
            );
    });

    bar.finish();
}

use std::fs::File;
use std::io::BufReader;
use std::path::Path;
use std::thread;

use bzip2::read::BzDecoder;
use crossbeam_channel::{bounded, Receiver};
use rayon::ThreadPoolBuilder;

pub async fn for_each_adsbx_json2<F, Fut>(file_paths: Vec<String>, mut closure: F)
where
    F: FnMut(adsbx_json::v2::Response) -> Fut + Send + Sync + 'static,
    Fut: std::future::Future<Output = ()> + Send + 'static,
{
    let file_paths = Arc::new(file_paths);
    let file_paths = Mutex::new(file_paths.iter());

    let (tx, rx) = bounded(1);
    let progress_bar = ProgressBar::new(file_paths.lock().unwrap().len() as u64);
    progress_bar.set_style(
        ProgressStyle::default_bar()
            .template(
                "{spinner:.green} [{elapsed_precise}] [{wide_bar:.cyan/blue}] {pos}/{len} ({eta})",
            )
            .progress_chars("#>-"),
    );

    ThreadPoolBuilder::new()
        .num_threads(4)
        .build()
        .unwrap()
        .install(|| {
            while let Some(file_path) = file_paths.lock().unwrap().next() {
                let data = load_adsbx_json(file_path).unwrap();
                tx.send(data).unwrap();
            }
        });

    let cloned_progress_bar = progress_bar.clone();

    let process_task = async move {
        while let Ok(data) = rx.recv() {
            closure(data).await;
            cloned_progress_bar.inc(1);
        }
    };

    process_task.await;
    progress_bar.finish();
}

pub fn for_each_adsbx_json_sync<OP>(paths: &[String], mut op: OP)
where
    OP: FnMut(adsbx_json::v2::Response) -> Option<String>,
{
    let bar = ProgressBar::new(paths.len().try_into().unwrap());
    bar.set_style(
        ProgressStyle::default_bar()
            .template("{wide_bar} {pos}/{len} {eta} {elapsed_precise} | {msg}"),
    );
    paths.iter().for_each(|path| {
        let result = load_adsbx_json(path);
        bar.inc(1);
        match result {
            Ok(data) => {
                let msg = op(data);
                if let Some(msg) = msg {
                    bar.set_message(msg);
                }
            }
            Err(e) => {
                eprintln!("Error loading {}: {}", path, e);
            }
        }
    });
    bar.finish();
}

/// Represents a bounding box. Used for filtering data to a region of interest.
#[derive(Debug, Clone, Copy)]
pub struct Bounds {
    pub min_lat: f32,
    pub min_lon: f32,
    pub max_lat: f32,
    pub max_lon: f32,
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
