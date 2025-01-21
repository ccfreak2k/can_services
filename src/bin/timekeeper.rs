use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use chrono::{DateTime, TimeDelta, Utc};
use clap::Parser;
use libc::{CLOCK_REALTIME, clock_settime, timespec};
use socketcan::{CanFilter, CanSocket, Socket, SocketOptions};

pub mod carlogger_service;

#[derive(Parser)]
#[command(name = "Timekeeper")]
#[command(version = "1.0")]
#[command(author)]
#[command(about = "Ensures local system clock is synced to GPS time")]

struct Args {
    #[arg(short = 'i', long, name = "name", default_value = "can0", help = "Interface to listen for traffic")]
    interface: String,
    #[arg(short = 'b', long, name = "speed", default_value = "500000", value_parser = clap::value_parser!(u64).range(1..), help = "The speed of the interface, in bps")]
    bus_speed: u64
}

fn main() {
    let matches = Args::parse();

    let interface: String = matches.interface;
    // Open the interface and set up a filter for frames with ID 0x465
    let can = CanSocket::open(&interface).unwrap();
    let filter = CanFilter::new(0x466, 0x7FF);
    can.set_filters(&[filter]).unwrap();
    can.set_read_timeout(Duration::from_secs(60)).unwrap();

    let bus_speed: u64 = matches.bus_speed;

    println!("Interface: {}", interface);
    println!("Bus speed: {}bps", bus_speed);

    let sig_term = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(signal_hook::consts::SIGTERM, Arc::clone(&sig_term)).unwrap();

    while !sig_term.load(Ordering::Relaxed) {
        match can.read_frame() {
            Ok(frame) => {
                let local_time: DateTime<Utc> = Utc::now();
                match carlogger_service::parse_frame(frame).unwrap() {
                    carlogger_service::ParsedFrame::_466(gps_time) => {
                        // GPS time is going to be slightly behind the real time by some fraction of a second
                        // due to CAN bus contention, but there's no way to measure it AFAIK besides assuming that the
                        // car clock is offset by the same amount. It should be close enough to not matter though.
                        // Compare the local clock to the GPS message and set it if it's more than 2 seconds off
                        if (local_time - gps_time).abs() > TimeDelta::seconds(2) {
                            println!("System time is {} seconds {} GPS time; setting system time",
                                (gps_time - local_time).num_seconds().abs() as f64,
                                if gps_time > local_time { "behind" } else { "ahead of" });
                            // Set the local system time to GPS time
                            let mut ts = timespec {
                                tv_sec: gps_time.timestamp() as i64,
                                tv_nsec: gps_time.timestamp_subsec_nanos() as i64,
                            };
                            let r = unsafe {
                                clock_settime(CLOCK_REALTIME, &mut ts)
                            };
                            if r != 0 {
                                panic!("Failed to set system time: {}", std::io::Error::last_os_error());
                            };
                        }
                    },
                    _ => ()
                }
            },
            Err(e) => {
                if socketcan::ShouldRetry::should_retry(&e) {
                    continue;
                } else if e.kind() == std::io::ErrorKind::Interrupted {
                    println!("Caught interrupt");
                    continue;
                } else {
                    panic!("Error reading from CAN bus: {}", e);
                }
            }
        }
    }
}
