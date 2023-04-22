use adsbx_json::v2::{Aircraft, AltitudeOrGround};
use serde_json::Value as JsonValue;
use structopt::lazy_static;
use thiserror::Error;

#[derive(Debug, Error)]
#[non_exhaustive]
pub enum Error {
    #[error("{0}")]
    AdsbxDbError(String),
}

macro_rules! define_cache_and_lookup {
    ($enum_type:ty, $table_name:ident) => {
        paste::paste! {
        lazy_static::lazy_static! {
            static ref [<CACHE_ $table_name:upper>]: std::sync::RwLock<std::collections::HashMap<$enum_type, i32>> =
                std::sync::RwLock::new(std::collections::HashMap::new());
        }

        // Generate a unique function name by appending the table name
        paste::paste! {
            async fn [<id_from_ $table_name>](
                client: &tokio_postgres::Client,
                value: $enum_type,
            ) -> Result<i32, Error> {
                // Check if the cache is empty. If it is, then fill it.
                {
                    let cache_read_guard = [<CACHE_ $table_name:upper>].read().unwrap();
                    if cache_read_guard.is_empty() {
                        drop(cache_read_guard);
                        let mut cache_write_guard = [<CACHE_ $table_name:upper>].write().unwrap();
                        if cache_write_guard.is_empty() {
                            for row in client
                                .query(
                                    &format!(r#"SELECT name, id FROM {}"#, stringify!($table_name)),
                                    &[],
                                )
                                .await
                                .map_err(|e| {
                                    Error::AdsbxDbError(format!("Error querying {} table: {}", stringify!($table_name), e))
                                })?
                            {
                                let value: $enum_type = serde_plain::from_str(row.get(0))
                                    .map_err(|e| {
                                    Error::AdsbxDbError(format!(
                                        "Error deserializing {} from database: {}",
                                        stringify!($table_name),
                                        e
                                    ))
                                })?;
                                let id: i32 = row.get(1);
                                cache_write_guard.insert(value, id);
                            }
                        }
                    }
                }
                // Now look up the value in the cache.
                let id: i32 = *[<CACHE_ $table_name:upper>]
                    .read()
                    .unwrap()
                    .get(&value)
                    .ok_or(Error::AdsbxDbError(format!(
                        "{} {:?} not found in cache",
                        stringify!($table_name),
                        value
                    )))?;
                Ok(id)
            }
        }
    }
    };
}

// Usage of the macro
define_cache_and_lookup!(adsbx_json::v2::Emergency, emergency);
define_cache_and_lookup!(adsbx_json::v2::MessageType, message_type);
define_cache_and_lookup!(adsbx_json::v2::NavMode, nav_mode);
define_cache_and_lookup!(adsbx_json::v2::SilType, sil_type);

pub async fn insert_aircraft(
    client: &tokio_postgres::Client,
    now: &chrono::DateTime<chrono::Utc>,
    aircraft: &Aircraft,
) -> Result<(), Error> {
    // Insert AcasRa if it exists
    let acas_ra_id: Option<i32> = if let Some(acas_ra) = &aircraft.acas_ra {
        client
            .query_one(
                r#"
        INSERT INTO acas_ra (
            ara, mte, rac, rat, tti,
            advisory, advisory_complement, bytes,
            threat_id_hex, unix_timestamp, utc
        ) VALUES (
            $1, $2, $3, $4, $5,
            $6, $7, $8, $9, $10,
            $11
        ) RETURNING id
        "#,
                &[
                    &acas_ra.ara,
                    &acas_ra.mte,
                    &acas_ra.rac,
                    &acas_ra.rat,
                    &acas_ra.tti,
                    &acas_ra.advisory,
                    &acas_ra.advisory_complement,
                    &acas_ra.bytes,
                    &acas_ra.threat_id_hex,
                    &acas_ra.unix_timestamp,
                    &acas_ra.utc,
                ],
            )
            .await
            .map_err(|e| Error::AdsbxDbError(format!("Error inserting AcasRa: {}", e)))?
            .get(0)
    } else {
        None
    };

    // // Insert LastPosition if it exists
    // let last_position_id: Option<i32> = if let Some(last_position) = &aircraft.last_position {
    //     client
    //         .query_one(
    //             r#"
    //     INSERT INTO last_position (
    //         seen_pos, lat, lon, nic,
    //         rc
    //     ) VALUES (
    //         $1, $2, $3, $4,
    //         $5
    //     ) RETURNING id
    //     "#,
    //             &[
    //                 &last_position.seen_pos,
    //                 &last_position.lat,
    //                 &last_position.lon,
    //                 &(last_position.nic as u32),
    //                 &last_position.rc,
    //             ],
    //         )
    //         .await?
    //         .get(0)
    // } else {
    //     None
    // };

    // Insert MlatFields if it exists
    if let Some(mlat_fields) = &aircraft.mlat_fields {
        // Insert each mlat field into the database.
        for mlat_field in mlat_fields {
            client
                .execute(
                    r#"
            INSERT INTO mlat_fields (
                mlat, tisb, tisb_id
            ) VALUES (
                $1, $2, $3
            )
            "#,
                    &[&mlat_field],
                )
                .await
                .map_err(|e| Error::AdsbxDbError(format!("Error inserting MlatFields: {}", e)))?;
        }
    }

    let emergency_id = if let Some(emergency) = aircraft.emergency {
        println!("emergency: {:?}", emergency);
        Some(id_from_emergency(client, emergency).await?)
    } else {
        None
    };
    let seen_timestamp =
        *now - chrono::Duration::milliseconds((aircraft.seen.as_secs_f64() * 1000.0) as i64);
    let seen_pos_timestamp = aircraft.seen_pos.as_ref().map(|seen_pos| {
        *now - chrono::Duration::milliseconds((seen_pos.as_secs_f64() * 1000.0) as i64)
    });
    // Convert barometric altitude to -9999 if it's ground.
    let barometric_altitude =
        aircraft
            .barometric_altitude
            .as_ref()
            .map(|baro_altitude| match baro_altitude {
                AltitudeOrGround::OnGround => &-9999,
                AltitudeOrGround::Altitude(altitude) => altitude,
            });
    let message_type_id = id_from_message_type(client, aircraft.message_type).await?;
    let sil_type_id = if let Some(sil_type) = aircraft.sil_type {
        Some(id_from_sil_type(client, sil_type).await?)
    } else {
        None
    };

    // Insert the Aircraft struct into the database, handling JSON serialization for enum types
    let aircraft_id: i32 = client
        .query_one(
            r#"
        INSERT INTO aircraft (
            acas_ra_id, adsb_version, aircraft_type, barometric_vertical_rate,
            barometric_altitude, calc_track, call_sign, database_flags,
            dir, distance_nm, emergency_id, emitter_category,
            geometric_altitude, geometric_vertical_accuracy, geometric_vertical_rate,
            gps_ok_before, gps_ok_lat, gps_ok_lon, ground_speed_knots,
            hex, indicated_air_speed_knots, is_alert, last_position_id,
            lat, lon, mach, magnetic_heading, message_type_id,
            nac_p, nac_v, nav_altitude_fms, nav_altitude_mcp,
            nav_heading, nav_qnh, nic, nic_baro,
            num_messages, outside_air_temperature, radius_of_containment_meters,
            registration, roll, rr_lat, rr_lon,
            rssi, seen, seen_pos, sil,
            sil_type_id, spi, squawk, system_design_assurance,
            total_air_temperature, track, track_rate, true_air_speed_knots,
            true_heading, wind_direction, wind_speed
        ) VALUES (
            $1, $2, $3, $4,
            $5, $6, $7, $8,
            $9, $10, $11, $12,
            $13, $14, $15,
            $16, $17, $18, $19,
            $20, $21, $22, $23,
            $24, $25, $26, $27,
            $28, $29, $30, $31, $32,
            $33, $34, $35, $36,
            $37, $38, $39, $40,
            $41, $42, $43,
            $44, $45, $46, $47,
            $48, $49, $50, $51,
            $52, $53, $54, $55,
            $56, $57, $58
        ) RETURNING id
        "#,
            &[
                &acas_ra_id,
                // Conert adsb_version to u32.
                &(aircraft.adsb_version.map(|v| v as i32)),
                &aircraft.aircraft_type,
                &aircraft.barometric_vertical_rate,
                &barometric_altitude,
                &(aircraft.calc_track.map(|v| v as i32)),
                &aircraft.call_sign,
                &(aircraft.database_flags.0 as i32),
                &aircraft.dir,
                &aircraft.distance_nm,
                &emergency_id,
                &aircraft.emitter_category,
                &aircraft.geometric_altitude,
                &(aircraft.geometric_vertical_accuracy.map(|v| v as i32)),
                &(aircraft.geometric_vertical_rate.map(|v| v as i32)),
                &aircraft.gps_ok_before,
                &aircraft.gps_ok_lat,
                &aircraft.gps_ok_lon,
                &aircraft.ground_speed_knots,
                &aircraft.hex,
                &aircraft.indicated_air_speed_knots,
                &aircraft.is_alert,
                // The next value is last_position_id:
                &None::<i32>,
                &aircraft.lat,
                &aircraft.lon,
                &aircraft.mach,
                &aircraft.magnetic_heading,
                &message_type_id,
                &(aircraft.nac_p.map(|v| v as i32)),
                &(aircraft.nac_v.map(|v| v as i32)),
                &aircraft.nav_altitude_fms,
                &aircraft.nav_altitude_mcp,
                &aircraft.nav_heading,
                &aircraft.nav_qnh,
                &(aircraft.nic.map(|v| v as i32)),
                &(aircraft.nic_baro.map(|v| v as i32)),
                &aircraft.num_messages,
                &aircraft.outside_air_temperature,
                &(aircraft.radius_of_containment_meters.map(|v| v as i32)),
                &aircraft.registration,
                &aircraft.roll,
                &aircraft.rr_lat,
                &aircraft.rr_lon,
                &aircraft.rssi,
                // seen:
                &seen_timestamp,
                // seen_pos:
                &seen_pos_timestamp,
                &(aircraft.sil.map(|v| v as i32)),
                &sil_type_id,
                &aircraft.spi,
                &aircraft.squawk,
                &(aircraft.system_design_assurance.map(|v| v as i32)),
                &aircraft.total_air_temperature,
                &aircraft.track,
                &aircraft.track_rate,
                &aircraft.true_air_speed_knots,
                &aircraft.true_heading,
                &(aircraft.wind_direction.map(|v| v as i32)),
                &(aircraft.wind_speed.map(|v| v as i32)),
            ],
        )
        .await
        .map_err(|e| {
            Error::AdsbxDbError(format!("Error inserting aircraft into database: {}", e))
        })?.get(0);
    println!("Inserted aircraft: {}", aircraft_id);
    // Get the ID of the inserted aircraft
    let aircraft_id: i32 = client
        .query_one("SELECT LASTVAL()", &[])
        .await
        .map_err(|e| {
            Error::AdsbxDbError(format!("Error getting aircraft ID from database: {}", e))
        })?
        .get(0);

    // Insert related data into corresponding tables

    // NavModes
    if let Some(nav_modes) = &aircraft.nav_modes {
        for nav_mode in nav_modes {
            let nav_mode_id: i32 = client
                .query_one(
                    "INSERT INTO nav_mode (mode) VALUES ($1) RETURNING id",
                    &[&serde_json::to_value(nav_mode).unwrap_or(JsonValue::Null)],
                )
                .await
                .map_err(|e| {
                    Error::AdsbxDbError(format!("Error inserting nav_mode into database: {}", e))
                })?
                .get(0);

            client
                .execute(
                    "INSERT INTO aircraft_nav_modes (aircraft_id, nav_mode_id) VALUES ($1, $2)",
                    &[&aircraft_id, &nav_mode_id],
                )
                .await
                .map_err(|e| {
                    Error::AdsbxDbError(format!(
                        "Error inserting aircraft_nav_modes into database: {}",
                        e
                    ))
                })?;
        }
    }

    // MlatFields
    if let Some(mlat_fields) = &aircraft.mlat_fields {
        for mlat_field in mlat_fields {
            client
                .execute(
                    "INSERT INTO aircraft_mlat_fields (aircraft_id, mlat_field) VALUES ($1, $2)",
                    &[&aircraft_id, &mlat_field],
                )
                .await
                .map_err(|e| {
                    Error::AdsbxDbError(format!(
                        "Error inserting aircraft_mlat_fields into database: {}",
                        e
                    ))
                })?;
        }
    }

    // TisbFields
    if let Some(tisb_fields) = &aircraft.tisb_fields {
        for tisb_field in tisb_fields {
            client
                .execute(
                    "INSERT INTO aircraft_tisb_fields (aircraft_id, tisb_field) VALUES ($1, $2)",
                    &[&aircraft_id, &tisb_field],
                )
                .await
                .map_err(|e| {
                    Error::AdsbxDbError(format!(
                        "Error inserting aircraft_tisb_fields into database: {}",
                        e
                    ))
                })?;
        }
    }

    Ok(())
}
