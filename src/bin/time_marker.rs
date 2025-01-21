use signal_hook::consts::TERM_SIGNALS;
use std::io::{self, Read, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use clap::Parser;

#[derive(Parser)]
#[command(name = "Time marker utility")]
#[command(version = "1.0")]
#[command(author)]
#[command(about = "Writes time markers with optional notes to a file")]

struct Args {
    #[arg(short = 'f', long, name = "file", help = "File to write time markers to")]
    file: PathBuf,
}

fn main() {
    let matches = Args::parse();

    let file_name: PathBuf = matches.file;

    let mut file = std::fs::OpenOptions::new().append(true).create(true).open(&file_name).unwrap();

    let sig = Arc::new(AtomicBool::new(false));
    for s in TERM_SIGNALS {
        signal_hook::flag::register_conditional_shutdown(*s, 1, Arc::clone(&sig)).unwrap();
        signal_hook::flag::register(*s, Arc::clone(&sig)).unwrap();
    }

    let mut stdin = io::stdin();
    println!("Press 'q' to quit, or any key to record a time marker");
    while !sig.load(Ordering::Relaxed) {
        
        let mut buf = [0; 1];
        let c = stdin.read(&mut buf).unwrap();
        if c == 0 {
            break;
        }
        match buf[0] {
            b'q' => {
                break;
            }
            _ => {
                // Write the full time as well as the unix timestamp with decimal
                let time = chrono::Local::now();
                let timestamp = time.timestamp() as f64 + time.timestamp_subsec_nanos() as f64 / 1_000_000_000.0;
                println!("Enter an optional note and press enter to write it to the file");
                let mut note = String::new();
                io::stdin().read_line(&mut note).unwrap();
                let line = format!("{} ({}) {}\n", time.format("%Y-%m-%d %H:%M:%S"), timestamp, note);
                file.write_all(line.as_bytes()).unwrap();
                println!("Time marker written to file");
            }
        }
    }
}