use log::info;
use std::time::Duration;
use std::io::{BufWriter, Result, Write};
use std::fs::{OpenOptions, File};
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
