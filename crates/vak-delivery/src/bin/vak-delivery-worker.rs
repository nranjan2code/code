//! Standalone presentation worker. Transport adapters will consume its
//! packets for external delivery adapters; this process only renders.

fn main() {
    std::process::exit(vak_delivery::worker::run_stdio());
}
