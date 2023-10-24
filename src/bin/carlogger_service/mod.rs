use log::info;
use std::time::{SystemTime, Duration, UNIX_EPOCH};
use std::io::{BufWriter, Write};
use std::fs::{OpenOptions, File};
use socketcan;

pub struct Service {
    socket: socketcan::CANSocket
}

impl Service {
    pub fn new(name: &str, id: u32, mask: u32) -> Service {
        info!("Carlogger service '{}' starting up", name);
        let s = socketcan::CANSocket::open("can0").unwrap();
        let filter: socketcan::CANFilter = socketcan::CANFilter::new(id, mask).unwrap();
        s.set_filter(&[filter]).unwrap();
        return Service { socket: s };
    }

    pub fn read_frame(&self) -> std::vec::Vec<u8> {
        let data = loop {
            let f = self.socket.read_frame().unwrap();
            if f.is_error() == false {
                break f.data().to_owned();
            }
        };
        return data;
    }

    pub fn write_frame(&self, id: u32, mut data: std::vec::Vec<u8>) {
        let f: socketcan::CANFrame = socketcan::CANFrame::new(id, data.as_mut_slice(), false, false).unwrap();
        self.socket.write_frame(&f).unwrap();
    }
}

pub struct Logger {
    fd: BufWriter<File>,
    iface: String,
    last_time_stamp: Duration,
}

impl Logger {
    pub fn new(path: String, iface: String, buf_size: usize) -> Logger {
        Logger {
            last_time_stamp: Duration::new(0, 0),
            iface,
            fd: BufWriter::with_capacity(buf_size, OpenOptions::new().append(true).create(true).open(path).unwrap())
        }
    }

    pub fn drop(&mut self) {
        let _ = self.fd.flush();
    }

    pub fn log(&mut self, f: socketcan::CANFrame) {
        self.last_time_stamp = std::cmp::max(SystemTime::now().duration_since(UNIX_EPOCH).unwrap(), self.last_time_stamp);
        let lts = self.last_time_stamp.as_micros();
        let header: String = format!("({}.{:06}) {}", lts/1_000_000, lts%1_000_000, self.iface);
        let body: String;
        if f.is_error() {
            // Just write a plain python canutils-style error to the log
            // TODO: have a separate log for logging the specific error if available
            //eprintln!("An error occurred: {}", f.err());
            body = format!("{} 20000080#0000000000000000\n", header);
        } else if f.is_rtr() {
            // Return request frame
            if f.is_extended() {
                body = format!("{} {:08X}#R\n", header, f.id());
            } else {
                body = format!("{} {:03X}#R\n", header, f.id());
            }
        } else {
            // Regular data frame
            if f.is_extended() {
                body = format!("{} {:08X}#{}\n", header, f.id(), hex::encode_upper(f.data()));
            } else {
                body = format!("{} {:03X}#{}\n", header, f.id(), hex::encode_upper(f.data()));
            }
        }
        self.fd.write(body.as_bytes()).unwrap();
    }

    pub fn flush(&mut self) {
        self.fd.flush().unwrap();
    }
}
