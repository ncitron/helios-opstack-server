use std::net::{IpAddr, Ipv4Addr, SocketAddr};
use alloy::primitives::address;
use kona_p2p::Network;
use libp2p::Multiaddr;
use tracing_subscriber::{EnvFilter, FmtSubscriber};

#[tokio::main]
async fn main() {
    enable_tracing();

    let signer = address!("AAAA45d9549EDA09E70937013520214382Ffc4A2");
    let chain_id = 10;

    let gossip = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), 14444);
    let mut gossip_addr = Multiaddr::from(gossip.ip());
    gossip_addr.push(libp2p::multiaddr::Protocol::Tcp(gossip.port()));
    let disc = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), 14445);

    let mut network = Network::builder()
        .with_chain_id(chain_id)
        .with_unsafe_block_signer(signer)
        .with_discovery_address(disc)
        .with_gossip_address(gossip_addr)
        .build()
        .expect("Failed to builder network driver");
    
    let mut block_recv = network.unsafe_block_recv();
    network.start().expect("Failed to start network driver");

    while let Ok(block) = block_recv.recv().await {
        let hash = block.payload.block_hash();
        println!("block received: {}", hash);
    }
}

fn enable_tracing() {
    let env_filter = EnvFilter::builder()
        .from_env()
        .expect("invalid env filter");

    let subscriber = FmtSubscriber::builder()
        .with_env_filter(env_filter)
        .finish();

    tracing::subscriber::set_global_default(subscriber).expect("subscriber set failed");
}
