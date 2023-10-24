use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;
use std::time;
use chrono;
use chrono::prelude::*;
use gpio::GpioOut;
use threadpool::ThreadPool;
use std::sync::mpsc::{self, Receiver, Sender};

mod carlogger_service;

extern crate clap;
extern crate socketcan;

enum LogMessage {
    Ping,
    Frame(socketcan::CANFrame),
    Flush,
    Exit,
}

fn is_num(v: String) -> Result<(), String> {
    let val: i64 = v.parse::<i64>().unwrap();
    if val > 0 { return Ok(()); }
    Err(String::from("Value must be a positive non-zero integer"))
}

fn main() {
    let matches = clap::App::new("Recorder")
        .version("1.0")
        .author("ccfreak2k")
        .about("Records CAN data to a file")
        .arg(clap::Arg::with_name("interface")
            .short("i")
            .long("interface")
            .value_name("IFACE")
            .help("Interface to listen for traffic")
            .takes_value(true)
            .default_value("can0"))
        .arg(clap::Arg::with_name("bus-speed")
            .short("b")
            .long("bus-speed")
            .value_name("BPS")
            .help("The speed of the interface, in bps")
            .takes_value(true)
            .validator(is_num)
            .default_value("500000"))
        .arg(clap::Arg::with_name("timeout-value")
            .short("t")
            .long("timeout-value")
            .value_name("SECONDS")
            .help("Number of seconds of bus silence allowed before the program will rotate logs")
            .takes_value(true)
            .validator(is_num)
            .default_value("15"))
        .arg(clap::Arg::with_name("log-location")
            .short("l")
            .long("log-location")
            .value_name("PATH")
            .help("The location to store the currently recording log")
            .takes_value(true)
            .default_value("."))
        .arg(clap::Arg::with_name("log-outbox")
            .short("o")
            .long("log-outbox")
            .value_name("PATH")
            .help("The location to move logs to when the logs are rotated")
            .takes_value(true)
            .required(true))
        .arg(clap::Arg::with_name("max-log-lines")
            .short("m")
            .long("max-log-lines")
            .value_name("LINES")
            .help("The maximum number of lines to record to a log file before the log is automatically rotated")
            .takes_value(true)
            .validator(is_num)
            .default_value("16777216"))
        .arg(clap::Arg::with_name("buffer-size")
            .short("s")
            .long("buffer-size")
            .value_name("SIZE")
            .help("The amount of bytes to buffer for file writes")
            .takes_value(true)
            .validator(is_num)
            .default_value("1048576"))
        .arg(clap::Arg::with_name("busy-led")
            .short("e")
            .long("busy-led")
            .value_name("PIN")
            .help("Which output GPIO pin to use for the busy LED. The LED will be lit as long as a log file is still open. Set to 0 to disable the LED function.")
            .takes_value(true)
            .validator(is_num)
            .default_value("22"))
        .get_matches();

    let interface: &str = matches.value_of("interface").unwrap();
    let can = socketcan::CANSocket::open(interface).unwrap();

    let timeout_value: u64 = matches.value_of("timeout-value").unwrap().parse::<u64>().unwrap();
    let bus_speed: u64     = matches.value_of("bus-speed").unwrap().parse::<u64>().unwrap();
    let log_location: &str = matches.value_of("log-location").unwrap();
    let log_outbox: &str   = matches.value_of("log-outbox").unwrap();
    let max_log_lines: u64 = matches.value_of("max-log-lines").unwrap().parse::<u64>().unwrap();
    let buffer_size: usize = matches.value_of("buffer-size").unwrap().parse::<usize>().unwrap();
    let busy_led_pin: u16  = matches.value_of("busy-led").unwrap().parse::<u16>().unwrap();

    println!("Interface:       {}", interface);
    println!("Bus speed:       {}", bus_speed);
    println!("Log location:    {}", log_location);
    println!("Outbox Location: {}", log_outbox);
    println!("Timeout value:   {}", timeout_value);
    println!("Max log lines:   {}", max_log_lines);
    println!("Write buffer:    {}", buffer_size);

    let term = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(signal_hook::consts::SIGTERM, Arc::clone(&term)).unwrap();
    let hup = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(signal_hook::consts::SIGHUP, Arc::clone(&hup)).unwrap();
    let mut busy_led = gpio::sysfs::SysFsGpioOutput::open(busy_led_pin).unwrap();

    // Two threads let one finish and close a file while the next starts a new one.
    let pool = ThreadPool::new(2);

    println!("Waiting for first frame");
    while !term.load(Ordering::Relaxed) {
        busy_led.set_low().unwrap();
        can.set_read_timeout(time::Duration::from_secs(60)).unwrap();
        can.filter_accept_all().unwrap();
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
            let st_iface: String = interface.to_string();
            // Pick up a new thread from the pool
            pool.execute(move|| {
                let mut logger = carlogger_service::Logger::new(log_path, st_iface, buffer_size);
                loop {
                    match rx.recv() {
                        Ok(message) => match message {
                            LogMessage::Ping => continue,
                            LogMessage::Frame(frame) => logger.log(frame),
                            LogMessage::Flush => logger.flush(),
                            LogMessage::Exit => break,
                        },
                        Err(_) => break,
                    };
                }
            });
            match tx.send(LogMessage::Frame(msg)) {
                Ok(_) => {},
                Err(_) => {
                    println!("Logging thread exited unexpectedly; rotating log");
                    continue;
                }
            };
            current_log_lines += 1;
            hup.store(false, Ordering::Relaxed);
            can.set_read_timeout(time::Duration::from_millis(500)).unwrap();
            let mut timeout: u64 = timeout_value*2;
            let mut busy_state: bool = false;
            let mut led_state: bool = false;
            let mut frame_counter: u32 = 0;
            while !hup.load(Ordering::Relaxed) && !term.load(Ordering::Relaxed) {
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
                match tx.send(LogMessage::Frame(msg)) {
                    Ok(_) => {},
                    Err(_) => {
                        println!("Logging thread exited unexpectedly; rotating log");
                        continue;
                    }
                };
                current_log_lines += 1;
                if current_log_lines >= max_log_lines {
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
            println!("Waiting for first frame");
        }
    }
}