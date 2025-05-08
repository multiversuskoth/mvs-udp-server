use crate::start_rollback_server;

#[tokio::main]
pub async fn main() {
    colog::init();
    start_rollback_server().await;
}
