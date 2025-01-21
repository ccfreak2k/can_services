This project implements CAN bus-related services. Currently only the logging module is implemented.

Recorder
---
The Recorder module receives CAN frames from the network and writes them to disk. It is similar to `candump` from cantools except it automatically detects bus activity and rolls the log file over.

shutdown_scheduler
---
The shutdown_scheduler examines CAN bus activity and creates a file with a future timestamp on the filesystem when the CAN bus activity goes quiet and the car is within a certain distance of a given point. This is to enable the recording system to be shut down once CAN bus activity has settled, which is best used when the car is parked at its main parking spot for the night.

timekeeper
---
This module is used to keep the recording system's clock disciplined with the car's GPS module. The system time will be set to GPS time if it drifts too far from GPS time. Useful during hot or cold conditions when the RTC or system clock may run too fast/slow.

clock_offset_viewers
---
This is used to show the absolute time difference between the system clock and the car's onboard clock and GPS time.

time_marker
---
This is used to create a note with a specific timestamp. Useful when attempting to correlate some specific car activity afterwards using the recorded logs.
