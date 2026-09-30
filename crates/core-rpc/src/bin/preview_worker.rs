//! The preview worker on its own, for the tests to start as a real process.
//! The app does not ship it: the app starts itself with the worker flag.

fn main() {
    core_rpc::preview::worker_main()
}
