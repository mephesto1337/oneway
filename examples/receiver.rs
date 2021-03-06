use std::env;

use oneway::connection::Server;
use oneway::udp::UdpReader;
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
    tracing::info!("config = {:?}", config);

    let socket = UdpSocket::bind(config.address).await?;
    tracing::info!("Waiting for new request");

    let mut server = Server::new_with_config(UdpReader::new(socket)?, config);
    tracing::trace!("server created");

    server.serve_forever().await?;

    Ok(())
}
