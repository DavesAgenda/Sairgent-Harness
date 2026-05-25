use rusqlite::Connection;
use sairgent_kernel::orchestrator::Orchestrator;
use sairgent_kernel::registry::Registry;

fn assert_send<T: Send>() {}
fn assert_sync<T: Sync>() {}

fn main() {
    assert_send::<Connection>();
    assert_send::<Registry>();
    assert_sync::<Registry>();
    assert_send::<Orchestrator>();
    assert_sync::<Orchestrator>();
}
