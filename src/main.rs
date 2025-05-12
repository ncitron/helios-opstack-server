use alloy::{
    primitives::{Address, Bytes, address},
    signers::Signature,
};
use axum::{Json, Router, extract::State, routing::get};
use clap::Parser;
use discv5::{ConfigBuilder, enr::CombinedKey};
use kona_p2p::{LocalNode, Network};
use kona_registry::ROLLUP_CONFIGS;
use libp2p::{Multiaddr, identity::Keypair};
use op_alloy_rpc_types_engine::{OpExecutionPayload, OpNetworkPayloadEnvelope};
use serde::{Deserialize, Serialize};
use ssz::Encode;
use std::{
    net::{IpAddr, Ipv4Addr, SocketAddr},
    sync::Arc,
};
use tokio::sync::RwLock;
use tracing::info;
use tracing_subscriber::{EnvFilter, FmtSubscriber};

#[tokio::main]
async fn main() {
    enable_tracing();
    let cli = Cli::parse();
    start(
        &cli.network,
        cli.disc_port,
        cli.gossip_port,
        cli.server_port,
    )
    .await;
}

#[derive(Parser)]
struct Cli {
    #[arg(short, long)]
    network: String,
    #[arg(short, long)]
    disc_port: u16,
    #[arg(long)]
    #[arg(short, long)]
    gossip_port: u16,
    #[arg(short, long)]
    server_port: u16,
}

async fn start(network: &str, disc_port: u16, gossip_port: u16, server_port: u16) {
    let chain_config = ChainConfig::from(network);

    let gossip = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), gossip_port);
    let mut gossip_addr = Multiaddr::from(gossip.ip());
    gossip_addr.push(libp2p::multiaddr::Protocol::Tcp(gossip.port()));

    let CombinedKey::Secp256k1(k256_key) = CombinedKey::generate_secp256k1() else {
        unreachable!()
    };
    let advertise_ip = IpAddr::V4(Ipv4Addr::UNSPECIFIED);
    let disc = LocalNode::new(k256_key, advertise_ip, disc_port, disc_port);
    let disc_listen = SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), disc_port);

    let gossip_key = Keypair::generate_secp256k1();

    let cfg = ROLLUP_CONFIGS
        .get(&chain_config.chain_id)
        .expect("rollup config not found")
        .clone();

    let mut network = Network::builder()
        .with_rollup_config(cfg)
        .with_unsafe_block_signer(chain_config.unsafe_signer)
        .with_discovery_address(disc)
        .with_gossip_address(gossip_addr)
        .with_keypair(gossip_key)
        .with_discovery_config(ConfigBuilder::new(disc_listen.into()).build())
        .build()
        .expect("Failed to builder network driver");

    let mut payload_recv = network.unsafe_block_recv();
    network
        .start()
        .await
        .expect("Failed to start network driver");

    let state = Arc::new(RwLock::new(ServerState {
        latest_commitment: None,
        chain_id: chain_config.chain_id,
    }));

    let state_copy = state.clone();

    tokio::spawn(async move {
        while let Ok(payload_envelope) = payload_recv.recv().await {
            let hash = payload_envelope.payload.block_hash();
            let number = payload_envelope.payload.block_number();
            info!("block received: {}", hash);

            let latest = state_copy
                .read()
                .await
                .latest_commitment
                .as_ref()
                .map(|value| value.1)
                .unwrap_or_default();

            if number > latest {
                let commitment = SequencerCommitment::from(payload_envelope);
                state_copy.write().await.latest_commitment = Some((commitment, number));
            }
        }
    });

    let router = Router::new()
        .route("/latest", get(latest_handler))
        .route("/chain_id", get(chain_id_handler))
        .with_state(state);

    let server_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(0, 0, 0, 0)), server_port);
    let listener = tokio::net::TcpListener::bind(server_addr).await.unwrap();
    axum::serve(listener, router).await.unwrap();
}

fn enable_tracing() {
    let env_filter =
        EnvFilter::from_default_env().add_directive("helios_opstack_server".parse().unwrap());

    let subscriber = FmtSubscriber::builder()
        .with_env_filter(env_filter)
        .finish();

    tracing::subscriber::set_global_default(subscriber).expect("subscriber set failed");
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct SequencerCommitment {
    data: Bytes,
    signature: Signature,
}

impl From<OpNetworkPayloadEnvelope> for SequencerCommitment {
    fn from(value: OpNetworkPayloadEnvelope) -> Self {
        let parent_root = value.parent_beacon_block_root.unwrap();
        let payload = match value.payload {
            OpExecutionPayload::V1(value) => value.as_ssz_bytes(),
            OpExecutionPayload::V2(value) => value.as_ssz_bytes(),
            OpExecutionPayload::V3(value) => value.as_ssz_bytes(),
            OpExecutionPayload::V4(value) => value.as_ssz_bytes(),
        };

        let data = [parent_root.as_slice(), &payload].concat();

        SequencerCommitment {
            data: data.into(),
            signature: value.signature,
        }
    }
}

#[derive(Clone)]
struct ServerState {
    latest_commitment: Option<(SequencerCommitment, u64)>,
    chain_id: u64,
}

async fn latest_handler(
    State(state): State<Arc<RwLock<ServerState>>>,
) -> Json<Option<SequencerCommitment>> {
    Json(
        state
            .read()
            .await
            .latest_commitment
            .as_ref()
            .map(|value| value.0.clone()),
    )
}

async fn chain_id_handler(State(state): State<Arc<RwLock<ServerState>>>) -> Json<u64> {
    Json(state.read().await.chain_id)
}

struct ChainConfig {
    unsafe_signer: Address,
    chain_id: u64,
}

impl From<&str> for ChainConfig {
    fn from(value: &str) -> Self {
        match value {
            "op-mainnet" => ChainConfig {
                unsafe_signer: address!("AAAA45d9549EDA09E70937013520214382Ffc4A2"),
                chain_id: 10,
            },
            "base" => ChainConfig {
                unsafe_signer: address!("Af6E19BE0F9cE7f8afd49a1824851023A8249e8a"),
                chain_id: 8453,
            },
            _ => panic!("network not recognized"),
        }
    }
}
