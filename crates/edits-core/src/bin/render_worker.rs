//! A bare render worker (see `edits_core::worker`), used by the engine tests.

fn main() {
    edits_core::worker::serve_if_requested();
}
