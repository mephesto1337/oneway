use oneway::connection::Client;
use oneway::tree::find_files;
use oneway::udp::UdpWriter;
use oneway::{Config, Result};

use tokio::net::UdpSocket;

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();
    let config = Config::from_file("client.conf")?;
    log::info!("config ={:?}", config);

    let config = config.clone();
    let socket = UdpSocket::bind("0.0.0.0:0").await?;
    socket.connect(&config.address).await?;

    let buf = [0u8; 1];
    socket.send(&buf[..]).await?;
    log::info!("Connected to {}", config.address);

    let mut client = Client::new_with_config(UdpWriter::new(socket)?, config);
    let files = find_files(".", false, |_| true)?;

    client.send_hello().await?;
    client.send_files(&files[..]).await?;
    client.send_done().await?;

    Ok(())
}
