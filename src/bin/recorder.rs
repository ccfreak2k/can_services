use std::convert::TryInto;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time;
use chrono;
use chrono::prelude::*;
use gpio::GpioOut;
use threadpool::Builder;
use std::sync::mpsc::{self, Receiver, Sender};

pub mod carlogger_service;

use clap::Parser;
use socketcan::{CanError, CanFrame, CanSocket, Socket, SocketOptions};

#[allow(dead_code)]
enum LogMessage {
    Ping,
    Frame(CanFrame),
    Flush,
    Exit,
}

#[allow(dead_code)]
enum WriterError {
    // Generic error
    Error(String),
    CANError(CanError),
    IOError(std::io::Error),
}

#[derive(Parser)]
#[command(name = "recorder")]
#[command(version = "1.1")]
#[command(author)]
#[command(about = "Records CAN data to a file")]
struct Args {
    #[arg(short = 'i', long, name = "name", default_value = "can0", help = "Interface to listen for traffic")]
    interface: String,
    #[arg(short = 'b', long, name = "speed", default_value = "500000", value_parser = clap::value_parser!(u64).range(1..), help = "The speed of the interface, in bps")]
    bus_speed: u64,
    #[arg(short = 't', long, name = "seconds", default_value = "15", value_parser = clap::value_parser!(u64).range(1..), help = "Number of seconds of bus silence allowed before the program will rotate logs")]
    timeout: u64,
    #[arg(short = 'l', long, name = "path", default_value = ".", help = "The location to store the currently recording log")]
    log_location: String,
    #[arg(short = 'm', long, name = "lines", default_value = "16777216", value_parser = clap::value_parser!(u64).range(1..), help = "The maximum number of lines to record to a log file before the log is automatically rotated")]
    max_log_lines: u64,
    #[arg(short = 's', long, name = "size", default_value = "1048576", value_parser = clap::value_parser!(u32).range(1..), help = "The amount of bytes to buffer for file writes")]
    buffer_size: u32,
    #[arg(short = 'e', long, name = "pin_number", default_value = "22", value_parser = clap::value_parser!(u16).range(0..), help = "Which output GPIO pin to use for the busy LED. The LED will be lit as long as a log file is still open. Set to 0 to disable the LED function.")]
    busy_led: u16,
}

fn main() {
    let matches = Args::parse();

    let interface: String = matches.interface;
    let can = CanSocket::open(&interface).unwrap();

    let timeout_value: u64 = matches.timeout;
    let bus_speed: u64     = matches.bus_speed;
    let log_location: &str = &matches.log_location;
    let max_log_lines: u64 = matches.max_log_lines;
    let buffer_size: usize = matches.buffer_size.try_into().unwrap();
    let busy_led_pin: u16  = matches.busy_led;

    println!("Interface:       {}", interface);
    println!("Bus speed:       {}", bus_speed);
    println!("Log location:    {}", log_location);
    println!("Timeout value:   {}", timeout_value);
    println!("Max log lines:   {}", max_log_lines);
    println!("Write buffer:    {}", buffer_size);

    let term = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(signal_hook::consts::SIGTERM, Arc::clone(&term)).unwrap();
    let hup = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(signal_hook::consts::SIGHUP, Arc::clone(&hup)).unwrap();
    let mut busy_led = gpio::sysfs::SysFsGpioOutput::open(busy_led_pin).unwrap();

    // Two threads let one finish and close a file while the next starts a new one.
    let pool = Builder::new().num_threads(2).thread_name("Writer".to_string()).build();

    println!("Waiting for first frame");
    while !term.load(Ordering::Relaxed) {
        if busy_led_pin != 0 {
            busy_led.set_low().unwrap();
        }
        can.set_read_timeout(time::Duration::from_secs(60)).unwrap();
        can.set_filter_accept_all().unwrap();
        // Wait for a CAN frame
        let mut current_log_lines: u64 = 0;

        let msg = match can.read_frame() {
            Ok(message) => message,
            Err(e) => {
                if !socketcan::ShouldRetry::should_retry(&e) && e.kind() != std::io::ErrorKind::Interrupted {
                    panic!("{}", e)
                } else {
                    // Read timed out; loop back
                    continue;
                }
            }
        };
        {
            // start logging
            if busy_led_pin != 0 {
                busy_led.set_high().unwrap();
            }
            let log_name = format!("{}.log", &Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true).replace(":","_"));
            let log_path = format!("{}/{}", log_location, log_name);
            println!("Logging to: {}", log_path);
            let (tx, rx): (Sender<LogMessage>, Receiver<LogMessage>) = mpsc::channel();
            let (etx, erx): (Sender<WriterError>, Receiver<WriterError>) = mpsc::channel();
            let st_iface: String = interface.to_string();
            // Pick up a new thread from the pool
            pool.execute(move|| {
                let mut logger = carlogger_service::Logger::new(log_path, st_iface, buffer_size);
                loop {
                    match rx.recv() {
                        Ok(message) => match message {
                            LogMessage::Ping => continue,
                            LogMessage::Frame(frame) => {
                                if let CanFrame::Error(ef) = frame {
                                    // Bubble up the error to the main thread but don't exit
                                    if let Err(_) = etx.send(WriterError::CANError(CanError::from(ef))) {
                                        break;
                                    }
                                }
                                match logger.log(frame) {
                                    Ok(s) => {
                                        if s == 0 {
                                            let _ = etx.send(WriterError::Error(String::from("Wrote 0 bytes to log")));
                                            break;
                                        }
                                    },
                                    Err(e) => {
                                        let _ = etx.send(WriterError::IOError(e));
                                        break;
                                    }
                                
                                };
                            }
                            LogMessage::Flush => {
                                match logger.flush() {
                                    Ok(_) => {},
                                    Err(e) => {
                                        let _ = etx.send(WriterError::IOError(e));
                                        break;
                                    }
                                };
                            },
                            LogMessage::Exit => {
                                break;
                            }
                        },
                        Err(_) => {
                            break;
                        },
                    };
                }
            });
            // An immediate failure to record a frame is basically unrecoverable, so just unwrap it
            tx.send(LogMessage::Frame(msg)).unwrap();
            current_log_lines += 1;
            hup.store(false, Ordering::Relaxed);
            can.set_read_timeout(time::Duration::from_millis(500)).unwrap();
            let mut timeout: u64 = timeout_value*2;
            let mut busy_state: bool = false;
            let mut led_state: bool = false;
            let mut frame_counter: u32 = 0;
            while !hup.load(Ordering::Relaxed) && !term.load(Ordering::Relaxed) {
                // Check the error queue first
                match erx.try_recv() {
                    Ok(e) => match e {
                        WriterError::Error(msg) => {
                            println!("Logging Error: {}", msg);
                            break;
                        },
                        WriterError::CANError(e) => {
                            println!("CAN Error: {}", e);
                        },
                        WriterError::IOError(e) => {
                            println!("IO Error: {}", e);
                            break;
                        }
                    },
                    Err(e) => {
                        match e {
                            mpsc::TryRecvError::Empty => {},
                            mpsc::TryRecvError::Disconnected => {
                                println!("Wrote {} lines to log", current_log_lines);
                                println!("Logging thread exited unexpectedly (thread feedback queue error); rotating log");
                                break;
                            }
                        }
                    },
                };
                let msg = match can.read_frame() {
                    Ok(message) => {
                        if busy_state == false && busy_led_pin != 0 {
                            busy_state = true;
                            frame_counter = 0;
                            led_state = true;
                            busy_led.set_high().unwrap();
                        }
                        // Flash the LED based on frame count
                        frame_counter += 1;
                        if frame_counter >= 100 && busy_led_pin != 0 {
                            frame_counter = 0;
                            led_state = !led_state;
                            busy_led.set_value(led_state).unwrap();
                        }
                        timeout = timeout_value*2;
                        message
                    },
                    Err(e) => {
                        if socketcan::ShouldRetry::should_retry(&e) {
                            busy_state = false;
                            frame_counter = 0;
                            if timeout == 0 {
                                break;
                            }
                            // Flash the LED based on timeout
                            if busy_led_pin != 0 {
                                if timeout % 2 == 0 {
                                    led_state = !led_state;
                                }
                                busy_led.set_value(led_state).unwrap();
                            }
                            timeout -= 1;
                            continue;
                        } else if e.kind() == std::io::ErrorKind::Interrupted {
                            // Interrupted by signal
                            continue;
                        } else {
                            // Some other unexpected unspecified thing happened
                            panic!("{}", e);
                        }
                    }
                };
                if let Err(_) = tx.send(LogMessage::Frame(msg)) {
                    println!("Wrote {} lines to log", current_log_lines);
                    println!("Logging thread exited unexpectedly (log queue sender error); rotating log");
                    break;
                }
                current_log_lines += 1;
                if current_log_lines >= max_log_lines {
                    println!("Wrote {} lines to log", current_log_lines);
                    println!("Max log lines reached; rotating log");
                    let _ = tx.send(LogMessage::Exit);
                    break;
                }
            }
            hup.store(false, Ordering::Relaxed);
            let _ = tx.send(LogMessage::Exit);
            if busy_led_pin != 0 {
                busy_led.set_low().unwrap();
            }
            println!("Wrote {} lines to log", current_log_lines);
            println!("Waiting for first frame");
        }
    }
}