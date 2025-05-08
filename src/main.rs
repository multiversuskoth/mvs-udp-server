use mvs_p2p_rollback_server::start_rollback_server;

#[tokio::main]
pub async fn main() {
    colog::init();
    start_rollback_server().await;
}
