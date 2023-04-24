use std::fmt::Binary;

use adsbx_json::v2::{Aircraft, AltitudeOrGround};
use futures::pin_mut;
use structopt::lazy_static;
use thiserror::Error;
use tokio::sync::RwLock;
use tokio_postgres::types::{ToSql, Type};
use tokio_postgres::{binary_copy::BinaryCopyInWriter, Client};

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
            static ref [<CACHE_ $table_name:upper>]: RwLock<std::collections::HashMap<$enum_type, i32>> =
                RwLock::new(std::collections::HashMap::new());
        }

        // Generate a unique function name by appending the table name
        paste::paste! {
            async fn [<id_from_ $table_name>](
                client: &tokio_postgres::Transaction<'_>,
                value: $enum_type,
            ) -> Result<i32, Error> {
                // Check if the cache is empty. If it is, then fill it.
                {
                    let cache_read_guard = [<CACHE_ $table_name:upper>].read().await;
                    if cache_read_guard.is_empty() {
                        drop(cache_read_guard);
                        let mut cache_write_guard = [<CACHE_ $table_name:upper>].write().await;
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
                    .await
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
define_cache_and_lookup!(adsbx_json::v2::Emergency, adsbx_emergency);
define_cache_and_lookup!(adsbx_json::v2::MessageType, adsbx_message_type);
define_cache_and_lookup!(adsbx_json::v2::NavMode, adsbx_nav_mode);
define_cache_and_lookup!(adsbx_json::v2::SilType, adsbx_sil_type);

pub async fn insert_aircraft(
    client: &tokio_postgres::Transaction<'_>,
    now: &chrono::DateTime<chrono::Utc>,
    aircraft: &Aircraft,
) -> Result<(), Error> {
    // Insert AcasRa if it exists
    let acas_ra_id: Option<i32> = if let Some(acas_ra) = &aircraft.acas_ra {
        client
            .query_one(
                r#"
        INSERT INTO adsbx_acas_ra (
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
    // if let Some(mlat_fields) = &aircraft.mlat_fields {
    //     // Insert each mlat field into the database.
    //     for mlat_field in mlat_fields {
    //         client
    //             .execute(
    //                 r#"
    //         INSERT INTO mlat_fields (
    //             mlat, tisb, tisb_id
    //         ) VALUES (
    //             $1, $2, $3
    //         )
    //         "#,
    //                 &[&mlat_field],
    //             )
    //             .await
    //             .map_err(|e| Error::AdsbxDbError(format!("Error inserting MlatFields: {}", e)))?;
    //     }
    // }

    let emergency_id = if let Some(emergency) = aircraft.emergency {
        // println!("emergency: {:?}", emergency);
        Some(id_from_adsbx_emergency(client, emergency).await?)
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
    let message_type_id = id_from_adsbx_message_type(client, aircraft.message_type).await?;
    let sil_type_id = if let Some(sil_type) = aircraft.sil_type {
        Some(id_from_adsbx_sil_type(client, sil_type).await?)
    } else {
        None
    };

    // Insert the Aircraft struct into the database, handling JSON serialization for enum types
    let aircraft_id: i32 = client
        .query_one(
            r#"
        INSERT INTO adsbx_aircraft (
            acas_ra_id, adsb_version, aircraft_type,
            barometric_altitude, call_sign,
            emergency_id,
            geometric_altitude,
            gps_ok_before, ground_speed_knots,
            hex,
            lat, lon,
            nac_p,
            nic,
            outside_air_temperature,
            registration, roll,
            seen,
            squawk,
            wind_direction, wind_speed
        ) VALUES (
            $1, $2, $3,
            $4, $5,
            $6,
            $7,
            $8, $9,
            $10,
            $11, $12,
            $13,
            $14,
            $15,
            $16, $17,
            $18,
            $19,
            $20, $21
        ) RETURNING id
        "#,
            &[
                &acas_ra_id,
                // Conert adsb_version to u32.
                &(aircraft.adsb_version.map(|v| v as i32)),
                &aircraft.aircraft_type,
                &barometric_altitude,
                &aircraft.call_sign,
                &emergency_id,
                &aircraft.geometric_altitude,
                &aircraft.gps_ok_before,
                &aircraft.ground_speed_knots,
                &aircraft.hex,
                &aircraft.lat,
                &aircraft.lon,
                &(aircraft.nac_p.map(|v| v as i32)),
                &(aircraft.nic.map(|v| v as i32)),
                &aircraft.outside_air_temperature,
                &aircraft.registration,
                &aircraft.roll,
                // seen:
                &seen_timestamp,
                &aircraft.squawk,
                &(aircraft.wind_direction.map(|v| v as i32)),
                &(aircraft.wind_speed.map(|v| v as i32)),
            ],
        )
        .await
        .map_err(|e| Error::AdsbxDbError(format!("Error inserting aircraft into database: {}", e)))?
        .get(0);
    // println!("Inserted aircraft: {}", aircraft_id);
    // Insert related data into corresponding tables

    // NavModes
    // if let Some(nav_modes) = &aircraft.nav_modes {
    //     for nav_mode in nav_modes {
    //         let nav_mode_id: i32 = client
    //             .query_one(
    //                 "INSERT INTO nav_mode (mode) VALUES ($1) RETURNING id",
    //                 &[&serde_json::to_value(nav_mode).unwrap_or(JsonValue::Null)],
    //             )
    //             .await
    //             .map_err(|e| {
    //                 Error::AdsbxDbError(format!("Error inserting nav_mode into database: {}", e))
    //             })?
    //             .get(0);

    //         client
    //             .execute(
    //                 "INSERT INTO aircraft_nav_modes (aircraft_id, nav_mode_id) VALUES ($1, $2)",
    //                 &[&aircraft_id, &nav_mode_id],
    //             )
    //             .await
    //             .map_err(|e| {
    //                 Error::AdsbxDbError(format!(
    //                     "Error inserting aircraft_nav_modes into database: {}",
    //                     e
    //                 ))
    //             })?;
    //     }
    // }

    // MlatFields
    // if let Some(mlat_fields) = &aircraft.mlat_fields {
    //     for mlat_field in mlat_fields {
    //         client
    //             .execute(
    //                 "INSERT INTO aircraft_mlat_fields (aircraft_id, mlat_field) VALUES ($1, $2)",
    //                 &[&aircraft_id, &mlat_field],
    //             )
    //             .await
    //             .map_err(|e| {
    //                 Error::AdsbxDbError(format!(
    //                     "Error inserting aircraft_mlat_fields into database: {}",
    //                     e
    //                 ))
    //             })?;
    //     }
    // }

    // TisbFields
    // if let Some(tisb_fields) = &aircraft.tisb_fields {
    //     for tisb_field in tisb_fields {
    //         client
    //             .execute(
    //                 "INSERT INTO aircraft_tisb_fields (aircraft_id, tisb_field) VALUES ($1, $2)",
    //                 &[&aircraft_id, &tisb_field],
    //             )
    //             .await
    //             .map_err(|e| {
    //                 Error::AdsbxDbError(format!(
    //                     "Error inserting aircraft_tisb_fields into database: {}",
    //                     e
    //                 ))
    //             })?;
    //     }
    // }

    Ok(())
}

pub async fn insert_adsbx_aircrafts(
    client: &mut Client,
    now: &chrono::DateTime<chrono::Utc>,
    aircrafts: &Vec<Aircraft>,
) -> Result<(), Error> {
    let tx = client
        .transaction()
        .await
        .map_err(|e| Error::AdsbxDbError(format!("Error creating transaction: {}", e)))?;
    let sink = tx
        .copy_in("COPY adsbx_aircraft (adsb_version, aircraft_type, barometric_altitude, call_sign, emergency_id, geometric_altitude, gps_ok_before, ground_speed_knots, hex, lat, lon, nac_p, nic, outside_air_temperature, registration, roll, seen, squawk, wind_direction, wind_speed) FROM STDIN BINARY")
        .await
        .map_err(|e| {
            Error::AdsbxDbError(format!("Error creating adsbx_aircraft copy sink: {}", e))
        })?;
    let col_types = vec![
        Type::INT2,
        Type::TEXT,
        Type::INT4,
        Type::TEXT,
        Type::INT2,
        Type::INT4,
        Type::TIMESTAMPTZ,
        Type::FLOAT4,
        Type::TEXT,
        Type::FLOAT4,
        Type::FLOAT4,
        Type::INT2,
        Type::INT2,
        Type::FLOAT4,
        Type::TEXT,
        Type::FLOAT4,
        Type::TIMESTAMPTZ,
        Type::TEXT,
        Type::INT2,
        Type::INT2,
    ];
    let writer = BinaryCopyInWriter::new(sink, &col_types);
    let num_written = write(writer, &now, &aircrafts).await;
    tx.commit()
        .await
        .map_err(|e| Error::AdsbxDbError(format!("Error committing transaction: {}", e)))?;
    Ok(())
}

async fn write(
    writer: BinaryCopyInWriter,
    now: &chrono::DateTime<chrono::Utc>,
    aircraft: &Vec<Aircraft>,
) {
    pin_mut!(writer);
    for aircraft in aircraft {
        let barometric_altitude =
            aircraft
                .barometric_altitude
                .as_ref()
                .map(|baro_altitude| match baro_altitude {
                    AltitudeOrGround::OnGround => &-9999,
                    AltitudeOrGround::Altitude(altitude) => altitude,
                });
        let emergency_id: Option<i16> = None;
        let seen_timestamp =
            *now - chrono::Duration::milliseconds((aircraft.seen.as_secs_f64() * 1000.0) as i64);
        writer
            .as_mut()
            .write(&[
                // Convert adsb_version to u32.
                &(aircraft.adsb_version.map(|v| v as i16)),
                &aircraft.aircraft_type,
                &barometric_altitude,
                &aircraft.call_sign,
                &emergency_id,
                &aircraft.geometric_altitude,
                &aircraft.gps_ok_before,
                &aircraft.ground_speed_knots,
                &aircraft.hex,
                &aircraft.lat,
                &aircraft.lon,
                &(aircraft.nac_p.map(|v| v as i16)),
                &(aircraft.nic.map(|v| v as i16)),
                &aircraft.outside_air_temperature,
                &aircraft.registration,
                &aircraft.roll,
                // seen:
                &seen_timestamp,
                &aircraft.squawk,
                &(aircraft.wind_direction.map(|v| v as i16)),
                &(aircraft.wind_speed.map(|v| v as i16)),
            ])
            .await
            .map_err(|e| {
                Error::AdsbxDbError(format!("Error inserting aircraft into database: {}", e))
            })
            .unwrap();
    }
    writer
        .finish()
        .await
        .map_err(|e| Error::AdsbxDbError(format!("Error inserting aircraft into database: {}", e)))
        .unwrap();
}
