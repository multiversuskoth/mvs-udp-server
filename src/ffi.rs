use std::net::UdpSocket;
use std::sync::atomic::{AtomicBool, Ordering};
use std::thread;

use crate::{get_mvsi_port, start_rollback_server};

// Global boolean to track UDP port availability
static PORT_AVAILABLE: AtomicBool = AtomicBool::new(true);

#[no_mangle]
pub extern "C" fn start_rollback_server_cpp() {
    // Use the global port variable to bind
    let port = get_mvsi_port();
    let socket = match UdpSocket::bind(format!("0.0.0.0:{}", port)) {
        Ok(sock) => sock,
        Err(_) => {
            PORT_AVAILABLE.store(false, Ordering::SeqCst);
            return; // Exit early if the port is not available
        }
    };

    // Unbind the socket before continuing
    drop(socket);

    thread::spawn(|| {
        let rt = tokio::runtime::Builder::new_current_thread()
            .worker_threads(2)
            .enable_all()
            .build()
            .unwrap();

        rt.block_on(start_rollback_server());
    });
}

#[no_mangle]
pub extern "C" fn is_port_open_cpp() -> bool {
    // Check if the port is available
    let available = PORT_AVAILABLE.load(Ordering::SeqCst);
    if available {
        return true;
    } else {
        return false;
    }
}