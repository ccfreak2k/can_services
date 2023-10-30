use socketcan::{CanFilter, CanSocket, Socket, SocketOptions, Frame, EmbeddedFrame};

use log::{info, warn, debug};

fn main() {
    info!("Starting up");
    // Open the interface and set the filter.
    let s = CanSocket::open("can0").unwrap();
    let filter: CanFilter = CanFilter::new(0x465, 0x7FF);
    s.set_filters(&[filter]).unwrap();
    loop {
        let f = s.read_frame().unwrap();
        // Process the frame
        if f.is_error_frame() == false {
            let data: &[u8] = f.data();
            match f.id_word() {
                0x084 => {
                    // Clock
                    let minute: u8 = data[4];
                    let second: u8 = data[5];
                    let hour: u8   = data[6];
                }
                0x465 => {
                    // Get the lat/lon as degrees only
                    // Data comes in as degrees, minutes, and minute fraction
                    let mut latitude: f32 = (data[0] - 89).into();
                    // minutes
                    latitude += ((data[1] >> 2 & 0x3F) as f32) / 60.;
                    // minutes frac
                    latitude += (((data[2] << 6) + (data[3] >> 2 & 0x3F)) as f32) * (0.0001 / 60.);
                    let mut longitude: f32 = ((data[4] << 1) + (data[5] >> 7 & 0x1)) as f32;
                    // minutes
                    longitude += ((data[5] >> 1 & 0x3F) as f32) / 60.;
                    // miunutes frac
                    longitude += (((data[6] << 6) + (data[7] >> 2 & 0x3F)) as f32) * (0.0001 / 60.);
                }
                0x472 => {
                    // Charging finish time estimate
                }
                0x473 => {
                    // Charging start time
                }
                _ => debug!("Ignoring frame")
            }
        } else {
            warn!("Ignored error frame");
        }
    }
}
