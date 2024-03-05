use log::info;
use std::time::Duration;
use std::io::{BufWriter, Result, Write};
use std::fs::{OpenOptions, File};

use geoutils::Location;
use socketcan::{CanFilter, CanFrame, CanSocket, EmbeddedFrame, Id, Socket, SocketOptions, Frame};

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

pub enum ParsedFrame {
    None,
    _465 (Location),
}

pub fn parse_frame(frame: CanFrame) -> ParsedFrame {
    match frame.id_word() {
        0x465 => {
            let data = frame.data();
            let lat: i16 = (data[0] as i16) - 89;
            let mut lat_minutes: f32 = ((data[1] >> 2) as u8) as f32;
            lat_minutes += ((u16::from_be_bytes([data[2], data[3]]) >> 2) as f32) * 0.0001;
            let lat_degrees = lat_minutes / 60.0;
            // if lat is less than 0, subtract lat_minutes
            // otherwise, add lat_minutes
            let latitude: f32 = if lat < 0 {
                lat as f32 - lat_degrees
            } else {
                lat as f32 + lat_degrees
            };
            let lon: i16 = (i16::from_be_bytes([data[4], data[5]]) >> 7) - 179;
            let mut lon_minutes: f32 = ((data[5] >> 1) & 0b111111) as f32;
            lon_minutes += ((u16::from_be_bytes([data[6], data[7]]) >> 2) as f32) * 0.0001;
            let lon_degrees = lon_minutes / 60.0;
            let longitude: f32 = if lon < 0 {
                lon as f32 - lon_degrees
            } else {
                lon as f32 + lon_degrees
            };
            return ParsedFrame::_465 (Location::new(latitude, longitude));
        },
        // Return nothing if there's no matches
        _ => return ParsedFrame::None,
    }
}