use anyhow::Result;
use bitcoin::{
    Transaction,
    consensus::{Decodable, Encodable},
    io::Cursor,
    p2p::{
        Address, Magic, ServiceFlags,
        message::{NetworkMessage, RawNetworkMessage},
        message_network::VersionMessage,
    },
};
use clap::{Parser, arg, command};
use futures::{StreamExt, stream::FuturesUnordered};
use std::{
    collections::HashSet,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    time::{Duration, SystemTime, UNIX_EPOCH},
};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpStream, lookup_host},
};
use tracing::info;

const NODE_LIBRE_RELAY: u64 = 1 << 29;

#[derive(Parser, Debug)]
#[command(author, version, about)]
struct Args {
    /// Hex-encoded raw transaction you’d like to blast
    #[arg(long)]
    tx: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().with_target(false).init();

    info!("Time to blast some nodes with pigeon poop! 🕊️💩");
    let args = Args::parse();
    let tx_string = args.tx;
    let tx = bitcoin::consensus::deserialize::<Transaction>(&hex::decode(tx_string.clone())?)?;
    let mut tx_bytes = Vec::new();
    RawNetworkMessage::new(Magic::BITCOIN, NetworkMessage::Tx(tx.clone()))
        .consensus_encode(&mut tx_bytes)?;

    let mut seed_addrs = Vec::new();

    let seeds: Vec<&'static str> = vec![
        "dnsseed.bluematt.me",
        "dnsseed.bitcoin.dashjr.org",
        "seed.bitcoinstats.com",
        "seed.btc.petertodd.org",
        "seed.bitcoin.sprovoost.nl",
        "dnsseed.emzy.de",
        "seed.bitcoin.wiz.biz",
    ];

    for seed in seeds {
        info!("fetching addrs from {:?}", seed);

        if let Ok(addrs) = lookup_host(format!("{}:8333", seed)).await {
            seed_addrs.extend(addrs.into_iter());
        }
    }

    info!("found {} seeds", seed_addrs.len());

    let mut libre = HashSet::<SocketAddr>::new();
    let mut tasks = FuturesUnordered::new();
    for a in seed_addrs {
        tasks.push(tokio::spawn(async move { crawl_seed(a).await }));
    }
    while let Some(Ok(list)) = tasks.next().await {
        if let Ok(addresses) = list {
            libre.extend(addresses);
        }
    }
    let peers: Vec<_> = libre.into_iter().filter(|a| a.is_ipv4()).collect();

    let mut poops = FuturesUnordered::new();
    for p in peers.clone() {
        let txb = tx_bytes.clone();
        poops.push(tokio::spawn(async move {
            let _ = poop_tx(p, &txb).await;
            p
        }));
    }

    info!("pooping on {} libre nodes", peers.len());

    while let Some(Ok(addr)) = poops.next().await {
        info!("you pooped on libre node {addr}");
    }

    info!(
        "tx {:?} blasted to {} libra nodes. GLHF",
        tx.compute_txid(),
        peers.len(),
    );

    Ok(())
}

fn build_version(addr: SocketAddr) -> VersionMessage {
    VersionMessage {
        version: 70016,
        services: ServiceFlags::from(NODE_LIBRE_RELAY),
        timestamp: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs() as i64,
        receiver: Address::new(&addr, ServiceFlags::NONE),
        sender: Address::new(
            &SocketAddr::new(IpAddr::V4(Ipv4Addr::UNSPECIFIED), 0),
            ServiceFlags::from(NODE_LIBRE_RELAY),
        ),
        nonce: 420,
        user_agent: "/Satoshi:29.0.0/".into(),
        start_height: 1337,
        relay: true,
    }
}

async fn read_msg<R: AsyncReadExt + Unpin>(r: &mut R) -> Result<RawNetworkMessage> {
    let mut hdr = [0u8; 24];
    r.read_exact(&mut hdr).await?;
    let len = u32::from_le_bytes(hdr[16..20].try_into().unwrap()) as usize;
    let mut payload = vec![0; len];
    r.read_exact(&mut payload).await?;
    Ok(RawNetworkMessage::consensus_decode(&mut Cursor::new(
        [hdr.to_vec(), payload].concat(),
    ))?)
}

async fn poop_tx(addr: SocketAddr, tx: &[u8]) -> Result<()> {
    let mut stream =
        tokio::time::timeout(Duration::from_millis(800), TcpStream::connect(addr)).await??;
    stream.set_nodelay(true)?;

    // send VERSION
    let mut buf = Vec::new();
    RawNetworkMessage::new(Magic::BITCOIN, NetworkMessage::Version(build_version(addr)))
        .consensus_encode(&mut buf)?;
    stream.write_all(&buf).await?;
    stream.flush().await?;

    let (mut rd, mut wr) = stream.split();
    // wait for their VERSION
    tokio::time::timeout(Duration::from_millis(800), async {
        loop {
            if let NetworkMessage::Version(_) = read_msg(&mut rd).await?.payload() {
                break Ok::<(), anyhow::Error>(());
            }
        }
    })
    .await??;

    // VERACK + TX
    buf.clear();
    RawNetworkMessage::new(Magic::BITCOIN, NetworkMessage::Verack).consensus_encode(&mut buf)?;
    wr.write_all(&buf).await?;
    wr.write_all(tx).await?;
    wr.flush().await?;
    Ok(())
}

async fn crawl_seed(seed: SocketAddr) -> Result<Vec<SocketAddr>> {
    let mut peers = Vec::new();
    let mut stream =
        match tokio::time::timeout(Duration::from_secs(3), TcpStream::connect(seed)).await {
            Ok(Ok(s)) => s,
            _ => return Ok(peers),
        };
    stream.set_nodelay(true)?;

    // send VERSION
    let mut buf = Vec::new();
    RawNetworkMessage::new(Magic::BITCOIN, NetworkMessage::Version(build_version(seed)))
        .consensus_encode(&mut buf)?;
    stream.write_all(&buf).await?;
    stream.flush().await?;

    let (mut rd, mut wr) = stream.split();

    loop {
        let msg = match tokio::time::timeout(Duration::from_secs(3), read_msg(&mut rd)).await {
            Ok(Ok(m)) => m,
            _ => break,
        };
        match *msg.payload() {
            NetworkMessage::Version(_) => {
                buf.clear();
                RawNetworkMessage::new(Magic::BITCOIN, NetworkMessage::Verack)
                    .consensus_encode(&mut buf)?;
                wr.write_all(&buf).await?;
                buf.clear();
                RawNetworkMessage::new(Magic::BITCOIN, NetworkMessage::GetAddr)
                    .consensus_encode(&mut buf)?;
                wr.write_all(&buf).await?;
                wr.flush().await?;
            }
            NetworkMessage::Addr(ref list) => {
                let flag = ServiceFlags::from(NODE_LIBRE_RELAY);
                for (_, a) in list {
                    if a.services.has(flag) {
                        if let Ok(sa) = a.socket_addr() {
                            peers.push(sa);
                        }
                    }
                }
                break;
            }
            _ => {}
        }
    }
    Ok(peers)
}
