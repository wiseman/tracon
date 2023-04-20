/// Detects aircrafts takeoffs from ADS-B data.
use adsbx_json::v2::AltitudeOrGround;
use geo::{prelude::Contains, Bearing, BoundingRect, CoordsIter, Simplify};
use log::debug;
use std::collections::HashMap;
// shapefile re-exports dbase so you can use it
use chrono::{prelude::*, Duration};
use dump::for_each_adsbx_json;
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
    alt: AltitudeOrGround,
}

/// What we keep track of for each aircraft.
#[derive(Default)]
struct AcState {
    recent_positions: Vec<Pos>,
}

/// Holds information about a detected takeoff.
struct Takeoff {
    /// Time of the takeoff.
    time: DateTime<Utc>,
    /// Approximate location of the takeoff.
    point: geo_types::Point<f64>,
    /// Approximate heading of the aircraft at takeoff.
    heading: f64,
}

#[derive(Default)]
struct AppState {
    aircraft: HashMap<String, AcState>,
    recent_takeoffs: HashMap<String, Takeoff>,
    num_takeoffs: usize,
}

trait TakingOff {
    fn taking_off(&self) -> Option<Takeoff>;
}

impl TakingOff for AcState {
    fn taking_off(&self) -> Option<Takeoff> {
        if self.recent_positions.len() < 5 {
            debug!("Not enough positions ({} < 5)", self.recent_positions.len());
            return None;
        }
        if !self
            .recent_positions
            .iter()
            .take(2)
            .all(|pos| pos.alt == AltitudeOrGround::OnGround)
        {
            debug!("First 2 positions are not on ground");
            debug!(
                "recent_positions={:?}",
                self.recent_positions.iter().take(2).collect::<Vec<_>>()
            );
            return None;
        }
        let mut i = 0;
        let mut consecutive_inc_alt_count = 0;
        while i + 2 < self.recent_positions.len() && i < 9 && consecutive_inc_alt_count < 3 {
            let alt_prev = &self.recent_positions[i + 1].alt;
            let alt_cur = &self.recent_positions[i + 2].alt;
            debug!("i:{} alt_prev={:?}, alt_cur={:?}", i, alt_prev, alt_cur);
            match (alt_prev, alt_cur) {
                (AltitudeOrGround::Altitude(alt_prev), AltitudeOrGround::Altitude(alt_cur)) => {
                    if alt_cur > alt_prev {
                        consecutive_inc_alt_count += 1;
                    } else {
                        consecutive_inc_alt_count = 0;
                    }
                }
                (AltitudeOrGround::OnGround, AltitudeOrGround::Altitude(_)) => {
                    consecutive_inc_alt_count = 1;
                }
                _ => {}
            }
            i += 1;
        }
        if consecutive_inc_alt_count < 3 {
            debug!(
                "Not enough consecutive increasing altitudes. i={}, consecutive_inc_alt_count={}",
                i, consecutive_inc_alt_count
            );
            return None;
        }
        debug!(
            "Found takeoff! i={}, consecutive_inc_alt_count={}",
            i, consecutive_inc_alt_count
        );
        // Compute heading from the last 3 positions
        let mut heading = self.recent_positions[1]
            .point
            .bearing(self.recent_positions[2].point);
        if heading < 0.0 {
            heading += 360.0;
        }
        Some(Takeoff {
            time: self.recent_positions[2].time,
            point: self.recent_positions[2].point,
            heading,
        })
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
                    alt: AltitudeOrGround::OnGround,
                },
                Pos {
                    time: Utc::now(),
                    point: geo_types::Point::new(0.0, 0.0),
                    alt: AltitudeOrGround::OnGround,
                },
                Pos {
                    time: Utc::now(),
                    point: geo_types::Point::new(0.0, 0.0),
                    alt: AltitudeOrGround::OnGround,
                },
                Pos {
                    time: Utc::now(),
                    point: geo_types::Point::new(0.0, 0.0),
                    alt: AltitudeOrGround::Altitude(1000),
                },
                Pos {
                    time: Utc::now(),
                    point: geo_types::Point::new(0.0, 0.0),
                    alt: AltitudeOrGround::Altitude(2000),
                },
            ],
        };
        assert!(ac_state.taking_off().is_none());
        ac_state.recent_positions.push(Pos {
            time: Utc::now(),
            point: geo_types::Point::new(0.0, 0.0),
            alt: AltitudeOrGround::Altitude(3000),
        });
        assert!(ac_state.taking_off().is_some());
    }

    #[test]
    fn test_taking_off2() {
        let ac_state = AcState {
            recent_positions: vec![
                Pos {
                    time: Utc::now(),
                    point: geo_types::Point::new(0.0, 0.0),
                    alt: AltitudeOrGround::OnGround,
                },
                Pos {
                    time: Utc::now(),
                    point: geo_types::Point::new(0.0, 0.0),
                    alt: AltitudeOrGround::OnGround,
                },
                Pos {
                    time: Utc::now(),
                    point: geo_types::Point::new(0.0, 0.0),
                    alt: AltitudeOrGround::Altitude(25),
                },
                Pos {
                    time: Utc::now(),
                    point: geo_types::Point::new(0.0, 0.0),
                    alt: AltitudeOrGround::Altitude(25),
                },
                Pos {
                    time: Utc::now(),
                    point: geo_types::Point::new(0.0, 0.0),
                    alt: AltitudeOrGround::Altitude(250),
                },
                Pos {
                    time: Utc::now(),
                    point: geo_types::Point::new(0.0, 0.0),
                    alt: AltitudeOrGround::Altitude(625),
                },
                Pos {
                    time: Utc::now(),
                    point: geo_types::Point::new(0.0, 0.0),
                    alt: AltitudeOrGround::Altitude(1100),
                },
            ],
        };
        assert!(ac_state.taking_off().is_some());
    }
}

fn main() -> Result<(), String> {
    // Init the env_logger and write to stdout.
    env_logger::Builder::from_default_env()
        .target(env_logger::Target::Stdout)
        .init();
    let args = CliArgs::from_args();

    let polygons: Vec<geo_types::MultiPolygon<f64>> =
        shapefile::read_as::<_, shapefile::Polygon, shapefile::dbase::Record>(
            "./cb_2018_us_nation_20m/cb_2018_us_nation_20m.shp",
        )
        .expect("Could not open polygon-shapefile")
        .iter()
        .map(|p| p.0.clone().into())
        .collect();
    // Compute the bounding box of the polygons.
    let polygon = polygons[0].clone();
    let simple_polygon = polygon.simplify(&0.05);
    let bbox = polygon.bounding_rect().unwrap();
    let min_lat = bbox.min().y;
    let min_lon = bbox.min().x;
    let max_lat = bbox.max().y;
    let max_lon = bbox.max().x;
    // Print how many polygons are in the shapefile.
    eprintln!("There are {} polygons in the shapefile", polygons.len());
    // Print the # of vertices in polygon and simple_polygon.
    eprintln!(
        "There are {} vertices in the polygon",
        polygon.coords_count()
    );
    eprintln!(
        "There are {} vertices in the simple_polygon",
        simple_polygon.coords_count()
    );

    let mut state = AppState::default();
    println!("time,lon,lat,hdg,url");

    for_each_adsbx_json(&args.paths, |adsbx_data| {
        // let date = adsbx_data.now.format("%Y-%m-%d").to_string();
        // let hour = adsbx_data.now.hour();
        adsbx_data.aircraft.iter().for_each(|ac| {
            // Check for lat and lon.
            if let (Some(lat), Some(lon), Some(alt)) = (ac.lat, ac.lon, &ac.barometric_altitude) {
                let geo_point = geo_types::Point::new(lon, lat);
                // Check if the point is within the min/max lat/lon:
                if geo_point.x() < min_lon || geo_point.x() > max_lon {
                    return;
                }
                if geo_point.y() < min_lat || geo_point.y() > max_lat {
                    return;
                }
                    // println!("{} is inside polygon {},{}", ac.hex);
                    let ac_state = state
                        .aircraft
                        .entry(ac.hex.clone())
                        .or_insert_with(AcState::default);
                    ac_state.recent_positions.push(Pos {
                        time: adsbx_data.now,
                        point: geo_point,
                        alt: alt.clone(),
                    });
                    // Keep only the last 5 minutes of positions for the aircraft.
                    ac_state.recent_positions.retain(|pos| {
                        adsbx_data.now - pos.time < Duration::minutes(5)
                    });
                    if let Some(takeoff) = state
                        .aircraft
                        .entry(ac.hex.clone())
                        .or_insert_with(AcState::default)
                        .taking_off()
                    {
                        // If the takeoff point is outside the polygon, ignore it.
                        if !simple_polygon.contains(&takeoff.point) {
                            return;
                        }
                        // Consider it a takeoff if either it isn't in
                        // recent_takeoffs, or it is in recent_takeoffs but was
                        // added more than 5 minutes ago.
                        if let Some(recent_takeoff) = state.recent_takeoffs.get(&ac.hex) {
                            if takeoff.time - recent_takeoff.time < Duration::minutes(5) {
                                return;
                            }
                        }
                        // Create an adsbx url that looks like
                        // https://globe.adsbexchange.com/?icao=<hex>>&lat=<lat>>&lon=<lon>&zoom=14&showTrace=YYYY-MM-DD&trackLabels&startTime=HH:MM&endTime=HH:MM
                        let url = format!(
                            "https://globe.adsbexchange.com/?icao={}&lat={}&lon={}&zoom=14&showTrace={}&trackLabels&startTime={}&endTime={}",
                            ac.hex,
                            takeoff.point.y(),
                            takeoff.point.x(),
                            takeoff.time.format("%Y-%m-%d"),
                            takeoff.time.format("%H:%M"),
                            (takeoff.time + Duration::minutes(5)).format("%H:%M")
                        );
                        println!(
                            "{},{},{},{},{},{}",
                            takeoff.time,
                            ac.hex,
                            takeoff.point.x(),
                            takeoff.point.y(),
                            takeoff.heading,
                            url
                        );
                        state.recent_takeoffs.insert(ac.hex.clone(), takeoff);
                        state.num_takeoffs += 1;
                    }
            }
        });
        Some(format!("{} takeoffs found", state.num_takeoffs))
    });
    // println!("{} inside, {} outside", state.num_inside, state.num_outside);
    Ok(())
}
