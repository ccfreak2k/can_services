use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time;
use chrono;
use chrono::prelude::*;
use gpio::GpioOut;

mod carlogger_service;

extern crate clap;
extern crate socketcan;

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
            .help("Which output GPIO pin to use for the busy LED. The LED will be lit as long as a log file is still open.")
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

    //let outbox_path = path::Path::new(log_outbox);
    //let log_path = path::Path::new(log_location);
    let term = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(signal_hook::consts::SIGTERM, Arc::clone(&term)).unwrap();
    let hup = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(signal_hook::consts::SIGHUP, Arc::clone(&hup)).unwrap();
    let mut busy_led = gpio::sysfs::SysFsGpioOutput::open(busy_led_pin).unwrap();

    println!("Waiting for first frame");
    while !term.load(Ordering::Relaxed) {
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
            busy_led.set_high().unwrap();
            let log_name = format!("{}.log", &Utc::now().to_rfc3339_opts(chrono::SecondsFormat::Secs, true).replace(":","_"));
            let log_path = format!("{}/{}", log_location, log_name);
            println!("Logging to {}", log_name);
            let mut logger = carlogger_service::Logger::new(log_path, interface.to_string(), buffer_size);
            logger.log(msg);
            current_log_lines += 1;
            hup.store(false, Ordering::Relaxed);
            //can.set_read_timeout(time::Duration::from_secs(timeout_value)).unwrap();
            can.set_read_timeout(time::Duration::from_millis(500)).unwrap();
            let mut timeout = timeout_value*2;
            let mut busy_state = 0;
            while !hup.load(Ordering::Relaxed) && !term.load(Ordering::Relaxed) {
                let msg = match can.read_frame() {
                    Ok(message) => {
                        if busy_state == 0 {
                            busy_state = 1;
                            busy_led.set_high().unwrap();
                        }
                        timeout = timeout_value;
                        message
                    },
                    Err(e) => {
                        if socketcan::ShouldRetry::should_retry(&e) {
                            busy_state = 0;
                            if timeout == 0 {
                                break;
                            }
                            if timeout % 2 == 0 {
                                busy_led.set_high().unwrap();
                            } else {
                                busy_led.set_low().unwrap();
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
                logger.log(msg);
                current_log_lines += 1;
                if current_log_lines >= max_log_lines {
                    println!("Max log lines reached; rotating log");
                    logger.flush();
                    break;
                }
            }
            hup.store(false, Ordering::Relaxed);
            logger.flush();
            busy_led.set_low().unwrap();
            println!("Waiting for first frame");
        }
    }
}