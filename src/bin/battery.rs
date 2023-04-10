mod carlogger_service;

// Service providing data about the battery

fn main() {
    let s = carlogger_service::Service::new("battery", 0x40A, 0x7FF);
    loop {
        let charge_level = s.read_frame()[5];
    }
}
