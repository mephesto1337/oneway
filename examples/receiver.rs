use oneway::connection::Server;
use oneway::udp::UdpReader;
use oneway::{Config, Result};

use tokio::net::UdpSocket;

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();
    let config = Config::from_file("client.conf")?;

    let config = config.clone();
    let socket = UdpSocket::bind(config.address).await?;
    log::info!("Waiting for new request");

    let mut server = Server::new_with_config(UdpReader::new(socket)?, config);
    log::trace!("server created");

    loop {
        server.serve_forever().await?;
    }
}
