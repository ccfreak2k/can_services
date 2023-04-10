 This project implements CAN bus-related services. Currently only the logging module is implemented.

 Recorder
 ---
 The Recorder module receives CAN frames from the network and writes them to disk. It is similar to `candump` from cantools except it automatically detects bus activity and rolls the log file over.
