-- MessageType Enum Table
DROP TABLE IF EXISTS message_type CASCADE;
CREATE TABLE message_type (
    id SERIAL PRIMARY KEY,
    name VARCHAR(32) NOT NULL UNIQUE
);
-- Possible values for message_type:
-- #[serde(rename = "adsb_icao")]
-- #[serde(rename = "adsb_icao_nt")]
-- #[serde(rename = "adsb_other")]
-- #[serde(rename = "adsc")]
-- #[serde(rename = "adsr_icao")]
-- #[serde(rename = "adsr_other")]
-- #[serde(rename = "mode_s")]
-- #[serde(rename = "mlat")]
-- #[serde(rename = "other")]
-- #[serde(rename = "tisb_icao")]
-- #[serde(rename = "tisb_other")]
-- #[serde(rename = "tisb_trackfile")]
-- #[serde(rename = "unknown")]
INSERT INTO message_type (id, name) VALUES (1, 'adsb_icao');
INSERT INTO message_type (id, name) VALUES (2, 'adsb_icao_nt');
INSERT INTO message_type (id, name) VALUES (3, 'adsb_other');
INSERT INTO message_type (id, name) VALUES (4, 'adsc');
INSERT INTO message_type (id, name) VALUES (5, 'adsr_icao');
INSERT INTO message_type (id, name) VALUES (6, 'adsr_other');
INSERT INTO message_type (id, name) VALUES (7, 'mode_s');
INSERT INTO message_type (id, name) VALUES (8, 'mlat');
INSERT INTO message_type (id, name) VALUES (9, 'other');
INSERT INTO message_type (id, name) VALUES (10, 'tisb_icao');
INSERT INTO message_type (id, name) VALUES (11, 'tisb_other');
INSERT INTO message_type (id, name) VALUES (12, 'tisb_trackfile');
INSERT INTO message_type (id, name) VALUES (13, 'unknown');


-- SilType Enum Table
DROP TABLE IF EXISTS sil_type CASCADE;
CREATE TABLE sil_type (
    id SERIAL PRIMARY KEY,
    name VARCHAR(16) NOT NULL UNIQUE
);
-- Possible values for sil_type:
-- #[serde(rename = "unknown")]
-- #[serde(rename = "perhour")]
-- #[serde(rename = "persample")]
INSERT INTO sil_type (id, name) VALUES (1, 'unknown');
INSERT INTO sil_type (id, name) VALUES (2, 'perhour');
INSERT INTO sil_type (id, name) VALUES (3, 'persample');


-- Emergency Enum Table
DROP TABLE IF EXISTS emergency CASCADE;
CREATE TABLE emergency (
    id INTEGER PRIMARY KEY,
    name VARCHAR(16) NOT NULL UNIQUE
);
-- Possible values for emergency
-- #[serde(rename = "general")]
-- #[serde(rename = "lifeguard")]
-- #[serde(rename = "minfuel")]
-- #[serde(rename = "nordo")]
-- #[serde(rename = "unlawful")]
-- #[serde(rename = "downed")]
-- #[serde(rename = "reserved")]
-- Insert values
INSERT INTO emergency (id, name) VALUES (0, 'none');
INSERT INTO emergency (id, name) VALUES (1, 'general');
INSERT INTO emergency (id, name) VALUES (2, 'lifeguard');
INSERT INTO emergency (id, name) VALUES (3, 'minfuel');
INSERT INTO emergency (id, name) VALUES (4, 'nordo');
INSERT INTO emergency (id, name) VALUES (5, 'unlawful');
INSERT INTO emergency (id, name) VALUES (6, 'downed');
INSERT INTO emergency (id, name) VALUES (7, 'reserved');


-- NavMode Enum Table
DROP TABLE IF EXISTS nav_mode CASCADE;
CREATE TABLE nav_mode (
    id SERIAL PRIMARY KEY,
    name VARCHAR(16) NOT NULL UNIQUE
);
-- Possible values for nav_mode:
-- #[serde(rename = "althold")]
-- #[serde(rename = "approach")]
-- #[serde(rename = "autopilot")]
-- #[serde(rename = "lnav")]
-- #[serde(rename = "tcas")]
-- #[serde(rename = "vnav")]
INSERT INTO nav_mode (id, name) VALUES (1, 'althold');
INSERT INTO nav_mode (id, name) VALUES (2, 'approach');
INSERT INTO nav_mode (id, name) VALUES (3, 'autopilot');
INSERT INTO nav_mode (id, name) VALUES (4, 'lnav');
INSERT INTO nav_mode (id, name) VALUES (5, 'tcas');
INSERT INTO nav_mode (id, name) VALUES (6, 'vnav');


-- AgedPosition Table
DROP TABLE IF EXISTS aged_position CASCADE;
CREATE TABLE aged_position (
    id SERIAL PRIMARY KEY,
    seen_pos DOUBLE PRECISION NOT NULL,
    lat DOUBLE PRECISION NOT NULL,
    lon DOUBLE PRECISION NOT NULL,
    nic INTEGER NOT NULL,
    rc INTEGER NOT NULL
);

-- AcasRa Table
DROP TABLE IF EXISTS acas_ra CASCADE;
CREATE TABLE acas_ra (
    id SERIAL PRIMARY KEY,
    ara VARCHAR(16) NOT NULL,
    mte VARCHAR(16) NOT NULL,
    rac VARCHAR(16) NOT NULL,
    rat VARCHAR(16) NOT NULL,
    tti VARCHAR(16) NOT NULL,
    advisory VARCHAR(128) NOT NULL,
    advisory_complement VARCHAR(128) NOT NULL,
    bytes VARCHAR(128) NOT NULL,
    threat_id_hex VARCHAR(128),
    unix_timestamp TIMESTAMP WITH TIME ZONE NOT NULL,
    utc VARCHAR(32) NOT NULL
);

-- Aircraft Table
DROP TABLE IF EXISTS aircraft CASCADE;
CREATE TABLE aircraft (
    id SERIAL PRIMARY KEY,
    acas_ra_id INTEGER REFERENCES acas_ra (id),
    adsb_version INTEGER,
    aircraft_type VARCHAR(16),
    barometric_vertical_rate INTEGER,
    barometric_altitude INTEGER,
    calc_track INTEGER,
    call_sign VARCHAR(32),
    database_flags INTEGER NOT NULL,
    dir DOUBLE PRECISION,
    distance_nm DOUBLE PRECISION,
    emergency_id INTEGER REFERENCES emergency (id),
    emitter_category VARCHAR(16),
    geometric_altitude INTEGER,
    geometric_vertical_accuracy INTEGER,
    geometric_vertical_rate INTEGER,
    gps_ok_before TIMESTAMP WITH TIME ZONE,
    gps_ok_lat DOUBLE PRECISION,
    gps_ok_lon DOUBLE PRECISION,
    ground_speed_knots DOUBLE PRECISION,
    hex VARCHAR(32) NOT NULL,
    indicated_air_speed_knots DOUBLE PRECISION,
    is_alert BOOLEAN,
    last_position_id INTEGER REFERENCES aged_position (id),
    lat DOUBLE PRECISION,
    lon DOUBLE PRECISION,
    mach DOUBLE PRECISION,
    magnetic_heading DOUBLE PRECISION,
    message_type_id INTEGER REFERENCES message_type (id),
    nac_p INTEGER,
    nac_v INTEGER,
    nav_altitude_fms INTEGER,
    nav_altitude_mcp INTEGER,
    nav_heading DOUBLE PRECISION,
    nav_qnh DOUBLE PRECISION,
    nic INTEGER,
    nic_baro INTEGER,
    num_messages INTEGER NOT NULL,
    outside_air_temperature DOUBLE PRECISION,
    radius_of_containment_meters INTEGER,
    registration VARCHAR(32),
    roll DOUBLE PRECISION,
    rr_lat DOUBLE PRECISION,
    rr_lon DOUBLE PRECISION,
    rssi DOUBLE PRECISION NOT NULL,
    seen TIMESTAMP WITH TIME ZONE NOT NULL,
    seen_pos TIMESTAMP WITH TIME ZONE,
    sil INTEGER,
    sil_type_id INTEGER REFERENCES sil_type (id),
    spi BOOLEAN,
    squawk VARCHAR(16),
    system_design_assurance INTEGER,
    total_air_temperature DOUBLE PRECISION,
    track DOUBLE PRECISION,
    track_rate DOUBLE PRECISION,
    true_air_speed_knots DOUBLE PRECISION,
    true_heading DOUBLE PRECISION,
    wind_direction INTEGER,
    wind_speed INTEGER
);

-- NavMode relation table
DROP TABLE IF EXISTS aircraft_nav_modes;
CREATE TABLE aircraft_nav_modes (
    aircraft_id INTEGER REFERENCES aircraft (id),
    nav_mode_id INTEGER REFERENCES nav_mode (id),
    PRIMARY KEY (aircraft_id, nav_mode_id)
);

-- MlatFields relation table
DROP TABLE IF EXISTS aircraft_mlat_fields;
CREATE TABLE aircraft_mlat_fields (
    aircraft_id INTEGER REFERENCES aircraft (id),
    mlat_field VARCHAR(32),
    PRIMARY KEY (aircraft_id, mlat_field)
);

-- TisbFields relation table
DROP TABLE IF EXISTS aircraft_tisb_fields;
CREATE TABLE aircraft_tisb_fields (
    aircraft_id INTEGER REFERENCES aircraft (id),
    tisb_field VARCHAR(32),
    PRIMARY KEY (aircraft_id, tisb_field)
);