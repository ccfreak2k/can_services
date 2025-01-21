use log::info;
use std::convert::TryInto;
use std::time::Duration;
use std::io::{BufWriter, Result, Write};
use std::fs::{OpenOptions, File};

use chrono::{DateTime, NaiveDate, NaiveDateTime, NaiveTime, TimeZone, Utc};
use geoutils::Location;
use socketcan::{CanFilter, CanFrame, CanSocket, EmbeddedFrame, Id, Socket, SocketOptions, Frame};
use uom::si::acceleration::meter_per_second_squared;
use uom::si::angle::degree;
use uom::si::angular_velocity::{radian_per_second, revolution_per_minute};
use uom::si::electric_potential::hectovolt;
use uom::si::f32::*;
use uom::si::length::{hectometer, kilometer};
use uom::si::power::watt;
use uom::si::velocity::mile_per_hour;


pub struct Service {
    socket: CanSocket,
}

impl Service {
    pub fn new(name: &str, id: u32, mask: u32) -> Service {
        info!("Carlogger service '{}' starting up", name);
        let s = CanSocket::open("can0").unwrap();
        let filter: CanFilter = CanFilter::new(id, mask);
        s.set_filters(&[filter]).unwrap();
        return Service { socket: s };
    }

    pub fn read_frame(&self) -> std::vec::Vec<u8> {
        let data = loop {
            let f = self.socket.read_frame().unwrap();
            if f.is_error_frame() == false {
                break f.data().to_owned()
            }
        };
        return data;
    }

    pub fn write_frame(&self, id: Id, mut data: std::vec::Vec<u8>) {
        let f: CanFrame = CanFrame::new(id, data.as_mut_slice()).unwrap();
        self.socket.write_frame(&f).unwrap();
    }
}

pub struct Logger {
    fd: BufWriter<File>,
    iface: String,
}

impl Logger {
    pub fn new(path: String, iface: String, buf_size: usize) -> Logger {
        Logger {
            iface,
            fd: BufWriter::with_capacity(buf_size, OpenOptions::new().append(true).create(true).open(path).unwrap())
        }
    }

    pub fn drop(&mut self) {
        let _ = self.fd.flush();
    }

    pub fn log(&mut self, f: CanFrame, t: Duration) -> Result<usize> {
        let lts = t.as_micros();
        let header: String = format!("({}.{:06}) {}", lts/1_000_000, lts%1_000_000, self.iface);
        let body: String = match f {
            CanFrame::Error { .. } => {
                // Just write a plain python canutils-style error to the log
                if f.is_extended() {
                    format!("{} {:08X}#{}\n", header, f.id_word(), hex::encode_upper(f.data()))
                } else {
                    format!("{} {:03X}#{}\n", header, f.id_word(), hex::encode_upper(f.data()))
                }
            },
            CanFrame::Remote { .. } => {
                // Return request frame
                if f.is_extended() {
                    format!("{} {:08X}#R\n", header, f.id_word())
                } else {
                    format!("{} {:03X}#R\n", header, f.id_word())
                }
            },
            CanFrame::Data { .. } => {
                // Regular data frame
                if f.is_extended() {
                    format!("{} {:08X}#{}\n", header, f.id_word(), hex::encode_upper(f.data()))
                } else {
                    format!("{} {:03X}#{}\n", header, f.id_word(), hex::encode_upper(f.data()))
                }
            },
        };
        return self.fd.write(body.as_bytes());
    }

    pub fn flush(&mut self) -> Result<()> {
        return self.fd.flush();
    }
}

pub enum CompassDirection {
    North,
    NorthEast,
    East,
    SouthEast,
    South,
    SouthWest,
    West,
    NorthWest,
}
pub enum ParsedFrame {
    _084 (NaiveDateTime),
    _091 {pitch: AngularVelocity, roll: AngularVelocity, yaw: AngularVelocity},
    _092 {lateral: Acceleration, longitudinal: Acceleration, vertical: Acceleration},
    _217 {fl: AngularVelocity, fr: AngularVelocity, rl: AngularVelocity, rr: AngularVelocity},
    _352 {electric_range: Length},
    _368 {ac_power_w: Power, other_power_w: Power},
    _37B {gas_range: Length},
    _430 {odometer: Length},
    _43D {accessory_battery_v: ElectricPotential},
    _465 (Location),
    _466 (DateTime<Utc>),
    _467 {direction: CompassDirection, compass_heading: Angle, gps_vehicle_speed: Velocity},
    _472 (NaiveDateTime),
    _473 (NaiveDateTime),
}

fn get_number(data: u64, offset: u8, size: u8) -> u64{
    return (data >> (64 - offset - size)) & ((1 << size) - 1);
}

/// Parses a CAN frame based on the arbitration ID. Returns a `ParsedFrame` if the ID is recognized.
pub fn parse_frame(frame: CanFrame) -> Option<ParsedFrame> {
    let data: u64 = u64::from_be_bytes(frame.data().try_into().unwrap());
    match frame.id_word() {
        0x084 => {
            // Local clock time
            let year: i32 = get_number(data, 0, 8) as i32 + 2000;
            let ordinal: u32 = get_number(data, 16, 16) as u32;
            let hour: u32 = get_number(data, 48, 8) as u32;
            let min: u32 = get_number(data, 32, 8) as u32;
            let sec: u32 = get_number(data, 40, 8) as u32;
            return Some(ParsedFrame::_084 (NaiveDateTime::new(NaiveDate::from_yo_opt(year, ordinal).unwrap(), NaiveTime::from_hms_opt(hour, min, sec).unwrap())));
        },
        0x091 => {
            // Gyroscope data
            let pitch: i16 = get_number(data, 7, 16) as i16;
            let roll: i16 = get_number(data, 23, 16) as i16;
            let yaw: i16 = get_number(data, 39, 16) as i16;
            return Some(ParsedFrame::_091 { pitch: AngularVelocity::new::<radian_per_second>((pitch as f32 - 6.5) / 10000.0), roll: AngularVelocity::new::<radian_per_second>((roll as f32 - 6.5) / 10000.0), yaw: AngularVelocity::new::<radian_per_second>((yaw as f32 - 6.5) / 10000.0) });
        },
        0x092 => {
            // Accelerometer data
            let lateral: i16 = get_number(data, 4, 13) as i16 - 40;
            let longitudinal: i16 = get_number(data, 20, 13) as i16 - 40;
            let vertical: i16 = get_number(data, 36, 13) as i16 - 40;
            return Some(ParsedFrame::_092 { lateral: Acceleration::new::<meter_per_second_squared>((lateral as f32) / 100.0), longitudinal: Acceleration::new::<meter_per_second_squared>((longitudinal as f32) / 100.0), vertical: Acceleration::new::<meter_per_second_squared>((vertical as f32) / 100.0) });
        },
        0x217 => {
            // Wheel rotation speed
            let fl: f32 = get_number(data, 0, 16) as f32 / 10.0;
            let fr: f32 = get_number(data, 16, 16) as f32 / 10.0;
            let rl: f32 = get_number(data, 32, 16) as f32 / 10.0;
            let rr: f32 = get_number(data, 48, 16) as f32 / 10.0;
            return Some(ParsedFrame::_217 { fl: AngularVelocity::new::<revolution_per_minute>(fl), fr: AngularVelocity::new::<revolution_per_minute>(fr), rl: AngularVelocity::new::<revolution_per_minute>(rl), rr: AngularVelocity::new::<revolution_per_minute>(rr) });
        },
        0x352 => {
            let electric_range: f32 = get_number(data, 12, 12) as f32;
            return Some(ParsedFrame::_352 {electric_range: Length::new::<hectometer>(electric_range)});
        },
        0x368 => {
            // Power usage
            let ac: f32 = (get_number(data, 6, 10) * 5) as f32;
            let other: f32 = (get_number(data, 38, 10) * 5) as f32;
            return Some(ParsedFrame::_368 { ac_power_w: Power::new::<watt>(ac), other_power_w: Power::new::<watt>(other) });
        },
        0x37B => {
            // Gas range
            let range: f32 = get_number(data, 48, 14) as f32;
            return Some(ParsedFrame::_37B {gas_range: Length::new::<hectometer>(range)});
        },
        0x430 => {
            // Odometer
            let distance: f32 = get_number(data, 8, 24) as f32;
            return Some(ParsedFrame::_430 {odometer: Length::new::<kilometer>(distance)});
        },
        0x43D => {
            // Accessory battery voltage
            let voltage: f32 = get_number(data, 48, 8) as f32;
            return Some(ParsedFrame::_43D {accessory_battery_v: ElectricPotential::new::<hectovolt>(voltage)});
        },
        0x465 => {
            // GPS position
            let lat: i16 = get_number(data, 0, 8) as i16 - 89;
            let mut lat_minutes: f32 = get_number(data, 8, 6) as f32;
            lat_minutes += (get_number(data, 16, 14) as f32) * 0.0001;
            let lat_mins = lat_minutes / 60.0;
            let latitude: f32 = if lat < 0 {
                lat as f32 - lat_mins
            } else {
                lat as f32 + lat_mins
            };
            let lon: i16 = get_number(data, 32, 9) as i16 - 179;
            let mut lon_minutes: f32 = get_number(data, 41, 6) as f32;
            lon_minutes += (get_number(data, 48, 14) as f32) * 0.0001;
            let lon_mins = lon_minutes / 60.0;
            let longitude: f32 = if lon < 0 {
                lon as f32 - lon_mins
            } else {
                lon as f32 + lon_mins
            };
            return Some(ParsedFrame::_465 (Location::new(latitude, longitude)));
        },
        0x466 => {
            // GPS time
            let hour: u32 = get_number(data, 0, 5) as u32;
            let min: u32 = get_number(data, 8, 6) as u32;
            let sec: u32 = get_number(data, 16, 6) as u32;
            let day: u32 = get_number(data, 34, 5) as u32 + 1;
            let month: u32 = get_number(data, 39, 5) as u32 + 1;
            let year: i32 = get_number(data, 45, 8) as i32 + 2010;
            return Some(ParsedFrame::_466 (Utc.with_ymd_and_hms(year, month, day, hour, min, sec).unwrap()));
        },
        0x467 => {
            // GPS heading/speed
            let direction: CompassDirection = match get_number(data, 17, 3) {
                0 => CompassDirection::North,
                1 => CompassDirection::NorthEast,
                2 => CompassDirection::East,
                3 => CompassDirection::SouthEast,
                4 => CompassDirection::South,
                5 => CompassDirection::SouthWest,
                6 => CompassDirection::West,
                7 => CompassDirection::NorthWest,
                _ => CompassDirection::North,
            };
            let heading: f32 = get_number(data, 24, 16) as f32 / 100.0;
            let speed: f32 = get_number(data, 40, 8) as f32; // MPH
            return Some(ParsedFrame::_467 {direction, compass_heading: Angle::new::<degree>(heading), gps_vehicle_speed: Velocity::new::<mile_per_hour>(speed)});
        },
        0x472 => {
            // Charge finish time
            let min: u32 = get_number(data, 24, 8) as u32;
            let hour: u32 = get_number(data, 32, 8) as u32;
            let day: u32 = get_number(data, 40, 8) as u32;
            let month: u32 = get_number(data, 48, 8) as u32;
            let year: i32 = get_number(data, 56, 8) as i32 + 2010;
            return Some(ParsedFrame::_472 (NaiveDateTime::new(NaiveDate::from_ymd_opt(year, month, day).unwrap(), NaiveTime::from_hms_opt(hour, min, 0).unwrap())));
        },
        0x473 => {
            // Charge start time
            let min: u32 = get_number(data, 24, 8) as u32;
            let hour: u32 = get_number(data, 32, 8) as u32;
            let day: u32 = get_number(data, 40, 8) as u32;
            let month: u32 = get_number(data, 48, 8) as u32;
            let year: i32 = get_number(data, 56, 8) as i32 + 2010;
            return Some(ParsedFrame::_473 (NaiveDateTime::new(NaiveDate::from_ymd_opt(year, month, day).unwrap(), NaiveTime::from_hms_opt(hour, min, 0).unwrap())));
        }
        // Return nothing if there's no matches
        _ => return None,
    }
}