use std::env;

use oneway::connection::Client;
use oneway::tree::find_files;
use oneway::udp::UdpWriter;
use oneway::{Config, Result};

use tokio::net::UdpSocket;

#[tokio::main]
async fn main() -> Result<()> {
    env_logger::init();

    let mut args = env::args();
    let progname = args.next().unwrap();
    let config_path = args
        .next()
        .expect(&format!("Usage: {} CONFIG_FILE", progname));

    let config = Config::from_file(config_path)?;
    log::info!("config = {:?}", config);

    let socket = UdpSocket::bind("0.0.0.0:0").await?;
    socket.connect(&config.address).await?;

    log::info!("Connected to {}", config.address);

    let files = find_files(&config.root, false, |_| true)?;
    let mut client = Client::new_with_config(UdpWriter::new(socket)?, config);

    client.send_hello().await?;
    client.send_files(&files[..]).await?;
    client.send_done().await?;

    Ok(())
}
