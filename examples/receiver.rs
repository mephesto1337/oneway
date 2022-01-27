use oneway::connection::Server;
use oneway::udp::UdpReader;
use oneway::{Config, Result};

use tokio::net::UdpSocket;

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();
    let config = Config::from_file("client.conf")?;

    loop {
        let config = config.clone();
        let socket = UdpSocket::bind(config.address).await?;
        log::info!("Waiting for new request");

        let mut buf = [0u8; 1];
        let (_, client_addr) = socket.recv_from(&mut buf[..]).await?;
        socket.connect(&client_addr).await?;
        log::info!("Got new client: {}", &client_addr);

        let mut server = Server::new_with_config(UdpReader::new(socket)?, config, client_addr);
        log::trace!("server created");

        if let Err(e) = server.serve_forever().await {
            log::error!("Issue when handling client {}: {}", server.client_addr(), e);
        }
    }
}
