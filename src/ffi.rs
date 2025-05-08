use std::thread;

use crate::start_rollback_server;

#[no_mangle]
pub extern "C" fn start_rollback_server_cpp() {
    colog::init();
    thread::spawn(|| {
        let rt = tokio::runtime::Builder::new_current_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();

        rt.block_on(start_rollback_server());
    });
}
