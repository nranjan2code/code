//! Standalone presentation worker. Transport adapters will consume its
//! packets in a later integration slice; this process only renders.

fn main() {
    std::process::exit(vak_delivery::worker::run_stdio());
}
