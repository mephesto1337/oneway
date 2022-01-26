use oneway::connection::Server;
use oneway::udp::UdpReader;
use oneway::{Config, Result};

use tokio::net::UdpSocket;

async fn handle_client(mut server: Server<UdpReader>) -> Result<()> {
    loop {
        let message = server.recv_message().await?;
        server.process_message(message).await;
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();
    let config = Config::from_file("client.conf")?;

    let config = config.clone();
    let socket = UdpSocket::bind(config.address).await?;
    log::info!("Waiting for new request");

    let mut buf = [0u8; 1];
    let (_, client_addr) = socket.recv_from(&mut buf[..]).await?;
    // socket.connect(&client_addr).await?;
    log::info!("Got new client: {}", client_addr);

    let server = Server::new_with_config(UdpReader::new(socket)?, config, client_addr.clone());
    tokio::spawn(async move {
        if let Err(e) = handle_client(server).await {
            log::error!("Issue when handling client {}: {}", client_addr, e);
        }
    });
    Ok(())
}
