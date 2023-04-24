-- MessageType Enum Table
DROP TABLE IF EXISTS adsbx_message_type CASCADE;
CREATE TABLE adsbx_message_type (
    id SERIAL PRIMARY KEY,
    name VARCHAR(32) NOT NULL UNIQUE
);
-- Possible values for adsbx_message_type:
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
INSERT INTO adsbx_message_type (id, name) VALUES (1, 'adsb_icao');
INSERT INTO adsbx_message_type (id, name) VALUES (2, 'adsb_icao_nt');
INSERT INTO adsbx_message_type (id, name) VALUES (3, 'adsb_other');
INSERT INTO adsbx_message_type (id, name) VALUES (4, 'adsc');
INSERT INTO adsbx_message_type (id, name) VALUES (5, 'adsr_icao');
INSERT INTO adsbx_message_type (id, name) VALUES (6, 'adsr_other');
INSERT INTO adsbx_message_type (id, name) VALUES (7, 'mode_s');
INSERT INTO adsbx_message_type (id, name) VALUES (8, 'mlat');
INSERT INTO adsbx_message_type (id, name) VALUES (9, 'other');
INSERT INTO adsbx_message_type (id, name) VALUES (10, 'tisb_icao');
INSERT INTO adsbx_message_type (id, name) VALUES (11, 'tisb_other');
INSERT INTO adsbx_message_type (id, name) VALUES (12, 'tisb_trackfile');
INSERT INTO adsbx_message_type (id, name) VALUES (13, 'unknown');


-- SilType Enum Table
DROP TABLE IF EXISTS adsbx_sil_type CASCADE;
CREATE TABLE adsbx_sil_type (
    id SERIAL PRIMARY KEY,
    name VARCHAR(16) NOT NULL UNIQUE
);
-- Possible values for adsbx_sil_type:
-- #[serde(rename = "unknown")]
-- #[serde(rename = "perhour")]
-- #[serde(rename = "persample")]
INSERT INTO adsbx_sil_type (id, name) VALUES (1, 'unknown');
INSERT INTO adsbx_sil_type (id, name) VALUES (2, 'perhour');
INSERT INTO adsbx_sil_type (id, name) VALUES (3, 'persample');


-- Emergency Enum Table
DROP TABLE IF EXISTS adsbx_emergency CASCADE;
CREATE TABLE adsbx_emergency (
    id INTEGER PRIMARY KEY,
    name VARCHAR(16) NOT NULL UNIQUE
);
-- Possible values for adsbx_emergency
-- #[serde(rename = "general")]
-- #[serde(rename = "lifeguard")]
-- #[serde(rename = "minfuel")]
-- #[serde(rename = "nordo")]
-- #[serde(rename = "unlawful")]
-- #[serde(rename = "downed")]
-- #[serde(rename = "reserved")]
-- Insert values
INSERT INTO adsbx_emergency (id, name) VALUES (0, 'none');
INSERT INTO adsbx_emergency (id, name) VALUES (1, 'general');
INSERT INTO adsbx_emergency (id, name) VALUES (2, 'lifeguard');
INSERT INTO adsbx_emergency (id, name) VALUES (3, 'minfuel');
INSERT INTO adsbx_emergency (id, name) VALUES (4, 'nordo');
INSERT INTO adsbx_emergency (id, name) VALUES (5, 'unlawful');
INSERT INTO adsbx_emergency (id, name) VALUES (6, 'downed');
INSERT INTO adsbx_emergency (id, name) VALUES (7, 'reserved');


-- NavMode Enum Table
DROP TABLE IF EXISTS adsbx_nav_mode CASCADE;
CREATE TABLE adsbx_nav_mode (
    id SERIAL PRIMARY KEY,
    name VARCHAR(16) NOT NULL UNIQUE
);
-- Possible values for adsbx_nav_mode:
-- #[serde(rename = "althold")]
-- #[serde(rename = "approach")]
-- #[serde(rename = "autopilot")]
-- #[serde(rename = "lnav")]
-- #[serde(rename = "tcas")]
-- #[serde(rename = "vnav")]
INSERT INTO adsbx_nav_mode (id, name) VALUES (1, 'althold');
INSERT INTO adsbx_nav_mode (id, name) VALUES (2, 'approach');
INSERT INTO adsbx_nav_mode (id, name) VALUES (3, 'autopilot');
INSERT INTO adsbx_nav_mode (id, name) VALUES (4, 'lnav');
INSERT INTO adsbx_nav_mode (id, name) VALUES (5, 'tcas');
INSERT INTO adsbx_nav_mode (id, name) VALUES (6, 'vnav');


-- AgedPosition Table
DROP TABLE IF EXISTS adsbx_aged_position CASCADE;
CREATE TABLE adsbx_aged_position (
    id SERIAL PRIMARY KEY,
    seen_pos DOUBLE PRECISION NOT NULL,
    lat DOUBLE PRECISION NOT NULL,
    lon DOUBLE PRECISION NOT NULL,
    nic INTEGER NOT NULL,
    rc INTEGER NOT NULL
);

-- AcasRa Table
DROP TABLE IF EXISTS adsbx_acas_ra CASCADE;
CREATE TABLE adsbx_acas_ra (
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
DROP TABLE IF EXISTS adsbx_aircraft CASCADE;
CREATE TABLE adsbx_aircraft (
    adsb_version SMALLINT,
    aircraft_type VARCHAR(16),
    barometric_altitude INTEGER,
    call_sign VARCHAR(32),
    emergency_id SMALLINT REFERENCES adsbx_emergency (id),
    geometric_altitude INTEGER,
    gps_ok_before TIMESTAMP WITH TIME ZONE,
    ground_speed_knots real,
    hex VARCHAR(32) NOT NULL,
    lat REAL,
    lon REAL,
    nac_p SMALLINT,
    nic SMALLINT,
    outside_air_temperature REAL,
    registration VARCHAR(32),
    roll REAL,
    seen TIMESTAMP WITH TIME ZONE NOT NULL,
    squawk VARCHAR(4),
    wind_direction SMALLINT,
    wind_speed SMALLINT
);

-- NavMode relation table
DROP TABLE IF EXISTS adsbx_aircraft_nav_modes;
CREATE TABLE adsbx_aircraft_nav_modes (
    aircraft_id INTEGER REFERENCES adsbx_aircraft (id),
    nav_mode_id INTEGER REFERENCES adsbx_nav_mode (id),
    PRIMARY KEY (aircraft_id, nav_mode_id)
);

-- MlatFields relation table
DROP TABLE IF EXISTS adsbx_aircraft_mlat_fields;
CREATE TABLE adsbx_aircraft_mlat_fields (
    aircraft_id INTEGER REFERENCES adsbx_aircraft (id),
    mlat_field VARCHAR(32),
    PRIMARY KEY (aircraft_id, mlat_field)
);

-- TisbFields relation table
DROP TABLE IF EXISTS adsbx_aircraft_tisb_fields;
CREATE TABLE adsbx_aircraft_tisb_fields (
    aircraft_id INTEGER REFERENCES adsbx_aircraft (id),
    tisb_field VARCHAR(32),
    PRIMARY KEY (aircraft_id, tisb_field)
);
