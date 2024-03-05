use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use clap::{CommandFactory, Parser};
use clap::error::ErrorKind;
use geoutils::{Location, Distance};
use socketcan::{CanFilter, CanSocket, Frame, Socket, SocketOptions};

pub mod carlogger_service;

#[derive(Parser)]
#[command(name = "shutdown scheduler")]
#[command(version = "1.0")]
#[command(author)]
#[command(about = "Writes the shutdown time to a file")]

struct Args {
    #[arg(short = 'i', long, name = "name", default_value = "can0", help = "Interface to listen for traffic")]
    interface: String,
    #[arg(short = 'b', long, name = "speed", default_value = "500000", value_parser = clap::value_parser!(u64).range(1..), help = "The speed of the interface, in bps")]
    bus_speed: u64,
    #[arg(short = 'a', long, name = "latitude", help = "Latitude of the centerpoint for the shutdown area, in degrees", allow_negative_numbers = true)]
    latitude: f32,
    #[arg(short = 'o', long, name = "longitude", help = "Longitude of the centerpoint for the shutdown area, in degrees", allow_negative_numbers = true)]
    longitude: f32,
    #[arg(short = 'r', long, name = "radius", help = "Radius of the shutdown area, in meters")]
    radius: f32,
    #[arg(short = 't', long, name = "time", default_value = "900", help = "Time to wait before shutting down, in seconds")]
    time: u64,
    #[arg(short = 'f', long, name = "file", default_value = "/tmp/shutdownat", help = "File to write the shutdown time to")]
    file: PathBuf,
    #[arg(short = 'd', long, name = "dry_run", help = "If specified, do not write the shutdown time to the file")]
    dry_run: bool,
}

fn main() {
    let matches = Args::parse();

    let interface: String = matches.interface;
    // Open the interface and set up a filter for frames with ID 0x465
    let can = CanSocket::open(&interface).unwrap();
    let filter = CanFilter::new(0x465, 0x7FF);
    can.set_filters(&[filter]).unwrap();
    can.set_read_timeout(Duration::from_secs(60)).unwrap();

    let bus_speed: u64 = matches.bus_speed;
    let latitude: f32 = matches.latitude;
    let longitude: f32 = matches.longitude;
    let radius: f32 = matches.radius;
    let time: u64 = matches.time;
    let file_name: PathBuf = matches.file;
    let dry_run: bool = matches.dry_run;

    if !file_name.parent().unwrap().exists() && dry_run == false {
        let mut cmd = Args::command();
        let error_msg = format!("Directory {} does not exist", file_name.parent().unwrap().to_str().unwrap());
        cmd.error(ErrorKind::ValueValidation, error_msg).exit();
    }

    println!("Interface: {}", interface);
    println!("Bus speed: {}", bus_speed);
    println!("Latitude:  {}", latitude);
    println!("Longitude: {}", longitude);
    println!("Radius:    {}m", radius);
    println!("Time:      {}s", time);
    println!("File:      {}", file_name.to_str().unwrap());
    println!("Dry run:   {}", dry_run);

    let sig_term = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(signal_hook::consts::SIGTERM, Arc::clone(&sig_term)).unwrap();

    let shutdown_position = Location::new(latitude, longitude);
    let mut last_position: Location = Location::new(0, 0);
    let mut last_time: Duration = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
    let mut update_last_position: bool = false;
    let mut print_waiting_message: bool = true;
    let mut has_left_shutdown_area: bool = false;

    while !sig_term.load(Ordering::Relaxed) {
        // Get the next frame
        if print_waiting_message == true {
            println!("Waiting for frame...");
            print_waiting_message = false;
        }
        match can.read_frame() {
            Ok(msg) => {
                if msg.id_word() == 0x465 {
                    update_last_position = true;
                    last_position = match carlogger_service::parse_frame(msg) {
                        Some(carlogger_service::ParsedFrame::_465(location)) => {
                            last_time = SystemTime::now().duration_since(UNIX_EPOCH).unwrap();
                            location
                        }
                        _ => last_position
                    };
                    if has_left_shutdown_area == false && last_position.is_in_circle(&shutdown_position, Distance::from_meters(radius)).unwrap() == false {
                        has_left_shutdown_area = true;
                    }
                } else {
                    println!("Received frame with ID 0x{:X}", msg.id_word());
                    continue;
                }
            },
            Err(e) => {
                if socketcan::ShouldRetry::should_retry(&e) {
                    // Update the shutdownat file as needed
                    if update_last_position == true {
                        update_last_position = false;
                        println!("Last location: {:?}", last_position);
                        println!("Distance to shutdown area: {}m", last_position.distance_to(&shutdown_position).unwrap().meters());
                        if last_position.is_in_circle(&shutdown_position, Distance::from_meters(radius)).unwrap() && has_left_shutdown_area == true {
                            let shutdown_at: u64 = last_time.as_secs() + time;
                            println!("Shutting down at {}", shutdown_at);
                            if dry_run == false {
                                std::fs::write(file_name.clone(), shutdown_at.to_string()).unwrap();
                            }
                        } else {
                            println!("Not in shutdown area; removing file");
                            // Check if the file exists first
                            if dry_run == false && file_name.exists() {
                                match std::fs::remove_file(file_name.clone()) {
                                    Ok(_) => (),
                                    Err(e) => match e.kind() {
                                        std::io::ErrorKind::NotFound => (),
                                        _ => panic!("Error removing file: {}", e)
                                    }
                                }
                            }
                        }
                        print_waiting_message = true;
                    }
                    continue;
                } else if e.kind() == std::io::ErrorKind::Interrupted {
                    println!("Caught interrupt");
                    continue;
                } else {
                    panic!("Error reading from CAN bus: {}", e);
                }
            }
        };

    }
    // Remove the file in case the service was stopped manually
    // This way it won't unexpectedly shut down.
    // If the program is terminated due to a system shutdown, it won't matter anyway.
    if dry_run == false {
        match std::fs::remove_file(file_name) {
            Ok(_) => (),
            Err(e) => match e.kind() {
                std::io::ErrorKind::NotFound => (),
                _ => panic!("Error removing file: {}", e)
            }
        }
    }
}