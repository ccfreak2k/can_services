use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::Duration;

use chrono::{DateTime, Utc};
use chrono_tz::Tz;
use clap::Parser;
use socketcan::{CanFilter, CanSocket, Socket, SocketOptions};

pub mod carlogger_service;

#[derive(Parser)]
#[command(name = "clock offset viewer")]
#[command(version = "1.0")]
#[command(author)]
#[command(about = "Shows the difference between the computer clock and the car/GPS clocks")]

struct Args {
    #[arg(short = 'i', long, name = "name", default_value = "can0", help = "Interface to listen for traffic")]
    interface: String,
    #[arg(short = 'b', long, name = "speed", default_value = "500000", value_parser = clap::value_parser!(u64).range(1..), help = "The speed of the interface, in bps")]
    bus_speed: u64,
    #[arg(short = 't', long, name = "timezone", help = "Timezone to assign to the car's local time; default is the system's timezone")]
    timezone: String,
}

fn main() {
    let matches = Args::parse();

    let timezone: Tz = if matches.timezone.is_empty() {
        // HACK: Get the timezone from /etc/timezone
        std::fs::read_to_string("/etc/timezone").unwrap().parse().unwrap()
    } else {
        matches.timezone.parse().unwrap()
    };

    let can = CanSocket::open(&matches.interface).unwrap();
    can.set_filters(&[CanFilter::new(0x084, 0x7FF),CanFilter::new(0x466, 0x7FF)]).unwrap();
    can.set_read_timeout(Duration::from_secs(60)).unwrap();

    println!("Interface: {}", matches.interface);
    println!("Bus speed: {}", matches.bus_speed);
    println!("Timezone:  {}", timezone.name());

    let sig_term = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(signal_hook::consts::SIGTERM, Arc::clone(&sig_term)).unwrap();

    while !sig_term.load(Ordering::Relaxed) {
        match can.read_frame() {
            Ok(frame) => {
                let local_time: DateTime<Utc> = Utc::now();
                match carlogger_service::parse_frame(frame).unwrap() {
                    carlogger_service::ParsedFrame::_084(car_time) => {
                        //let car_time = car_time.and_local_timezone(FixedOffset::east_opt(matches.offset).unwrap()).unwrap();
                        // Apply the timezone offset to car_time
                        //let car_time = car_time.and_local_timezone(FixedOffset::east_opt(matches.offset*3600).unwrap()).unwrap();
                        let car_time = car_time.and_local_timezone(timezone).unwrap();
                        println!("Car time is {} seconds from local time", car_time.signed_duration_since(local_time).num_nanoseconds().unwrap() as f64 / 1_000_000_000.0);
                        println!("Car time is {}", car_time.to_string());
                    },
                    carlogger_service::ParsedFrame::_466(gps_time) => {
                        println!("GPS time is {} seconds from local time", gps_time.signed_duration_since(local_time).num_nanoseconds().unwrap() as f64 / 1_000_000_000.0);
                        println!("GPS time is {}", gps_time.to_string());
                    },
                    _ => {}
                }
            },
            Err(e) => {
                if socketcan::ShouldRetry::should_retry(&e) {
                    continue;
                } else if e.kind() == std::io::ErrorKind::Interrupted {
                    // Interrupted by signal
                    continue;
                } else {
                    panic!("{}", e);
                }
            }
        }
    }
}