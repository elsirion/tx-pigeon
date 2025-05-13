use anyhow::Result;
use axum::extract::ConnectInfo;
use axum::response::IntoResponse;
use axum::{
    Router,
    extract::{Form, State},
    response::Html,
    routing::{get, post},
};
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
use clap::{Parser, arg};
use futures::{StreamExt, stream::FuturesUnordered};
use maud::{html, Markup, PreEscaped, DOCTYPE};
use once_cell::sync::Lazy;
use serde::Deserialize;
use std::future::ready;
use std::net::IpAddr;
use std::{collections::HashMap, sync::Arc, time::Instant};
use std::{net::SocketAddr, time::Duration};
use tokio::net::TcpListener;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::{TcpStream, lookup_host},
};

const NODE_LIBRE_RELAY: u64 = 1 << 29;

#[derive(Parser, Debug, Clone)]
struct AppConfig {
    /// Site name for the web UI
    #[arg(long, env = "SITE_NAME", default_value = "Tx Pigeon")]
    site_name: String,

    /// Rate limit per minute per IP
    #[arg(long, env = "RATE_LIMIT_PER_MINUTE", default_value_t = 5)]
    rate_limit_per_minute: u32,
}

// --- Node Cache ---
static NODE_CACHE: Lazy<tokio::sync::RwLock<NodeCache>> =
    Lazy::new(|| tokio::sync::RwLock::new(NodeCache::default()));

#[derive(Default)]
struct NodeCache {
    nodes: Vec<SocketAddr>,
    last_refresh: Option<Instant>,
}

// --- Rate Limiting ---
static RATE_LIMITS: Lazy<tokio::sync::Mutex<HashMap<IpAddr, (u32, Instant)>>> =
    Lazy::new(|| tokio::sync::Mutex::new(HashMap::new()));

// --- Web Handlers ---
async fn index(State(cfg): State<Arc<AppConfig>>) -> impl IntoResponse {
    Html(render_form(&cfg.site_name).await.into_string())
}

#[derive(Deserialize)]
struct TxForm {
    tx: String,
}

fn check_rate_limit(ip: IpAddr, cfg: &AppConfig) -> Result<(), &'static str> {
    let mut limits = RATE_LIMITS.blocking_lock();
    let now = Instant::now();
    let entry = limits.entry(ip).or_insert((0, now));
    if now.duration_since(entry.1) > Duration::from_secs(60) {
        *entry = (0, now);
    }
    if entry.0 >= cfg.rate_limit_per_minute {
        return Err("Rate limit exceeded. Try again later.");
    }
    entry.0 += 1;
    Ok(())
}

async fn submit_tx(
    State(cfg): State<Arc<AppConfig>>,
    ConnectInfo(addr): axum::extract::ConnectInfo<SocketAddr>,
    Form(form): Form<TxForm>,
) -> Html<String> {
    // Rate limit check (per IP)
    if let Err(msg) = tokio::task::block_in_place(|| check_rate_limit(addr.ip(), &cfg)) {
        return Html(render_form_with_msg(&cfg.site_name, Some(msg)).await.into_string());
    }

    // Validate and blast tx
    let tx_bytes = match hex::decode(&form.tx) {
        Ok(bytes) => bytes,
        Err(_) => {
            return Html(
                render_form_with_msg(&cfg.site_name, Some("Invalid hex string")).await.into_string(),
            );
        }
    };
    let tx: Transaction = match bitcoin::consensus::deserialize(&tx_bytes) {
        Ok(tx) => tx,
        Err(_) => {
            return Html(
                render_form_with_msg(&cfg.site_name, Some("Invalid transaction encoding"))
                    .await.into_string(),
            );
        }
    };
    let mut tx_bytes = Vec::new();
    RawNetworkMessage::new(Magic::BITCOIN, NetworkMessage::Tx(tx.clone()))
        .consensus_encode(&mut tx_bytes)
        .expect("Encoding to Vec<u8> can't fail");

    // Get cached nodes
    let nodes = {
        let cache = NODE_CACHE.read().await;
        cache.nodes.clone()
    };

    if nodes.is_empty() {
        return Html(
            render_form_with_msg(
                &cfg.site_name,
                Some("No nodes available to blast to (cache empty)!"),
            )
            .await.into_string(),
        );
    }

    let total_nodes = nodes.len();
    let poops = nodes
        .into_iter()
        .map(|node| poop_tx(node, &tx_bytes))
        .collect::<FuturesUnordered<_>>();

    let successes = poops.filter_map(|res| ready(res.ok())).count().await;

    let msg = html! {
        a href={ (format!("https://mempool.space/tx/{}", tx.compute_txid())) }
          class="inline-block underline text-blue-700 hover:text-blue-900 text-sm" target="_blank" {
            "Transaction successfully blasted to " (successes) " out of " (total_nodes) " nodes. GLHF"
        }
    };
    Html(render_form_with_msg(&cfg.site_name, Some(&msg.into_string())).await.into_string())
}

// --- HTML Rendering ---
async fn render_form(site_name: &str) -> Markup {
    render_form_with_msg(site_name, None).await
}

async fn render_form_with_msg(site_name: &str, msg: Option<&str>) -> Markup {
    let total_nodes = NODE_CACHE.read().await.nodes.len();
    html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="utf-8";
                meta name="viewport" content="width=device-width, initial-scale=1";
                title { (site_name) }
                // Flowbite + Tailwind CDN
                link rel="stylesheet" href="https://cdnjs.cloudflare.com/ajax/libs/flowbite/2.3.0/flowbite.min.css";
            }
            body class="bg-gray-50 dark:bg-gray-900 min-h-screen flex flex-col items-center justify-center" {
                div class="w-full max-w-md p-8 bg-white rounded-lg shadow-xl dark:bg-gray-800 mt-10" {
                    h1 class="mb-4 text-3xl font-extrabold text-gray-900 dark:text-white text-center" { (site_name) }
                    p class="text-center text-gray-500 dark:text-gray-400" { "Blast your transaction to " (total_nodes) " libre relay nodes" }
                    @if let Some(msg) = msg {
                        div class="mb-4 p-2 bg-blue-100 text-blue-800 rounded mt-4" { (PreEscaped(msg)) }
                    }
                    form method="post" action="/submit" class="space-y-6 mt-6" {
                        textarea name="tx" id="tx" required rows="6" placeholder="Paste hex transaction..." class="bg-gray-50 border border-gray-300 text-gray-900 text-sm rounded-lg focus:ring-blue-500 focus:border-blue-500 block w-full p-2.5 font-mono break-words break-all dark:bg-gray-700 dark:border-gray-600 dark:placeholder-gray-400 dark:text-white dark:focus:ring-blue-500 dark:focus:border-blue-500" {}
                        button type="submit" class="w-full text-white bg-blue-700 hover:bg-blue-800 focus:ring-4 focus:outline-none focus:ring-blue-300 font-medium rounded-lg text-sm px-5 py-2.5 text-center dark:bg-blue-600 dark:hover:bg-blue-700 dark:focus:ring-blue-800" { "Blast Transaction" }
                    }
                }
            }
        }
    }
}

fn build_version(addr: SocketAddr) -> VersionMessage {
    VersionMessage {
        version: 70016,
        services: ServiceFlags::from(NODE_LIBRE_RELAY),
        timestamp: chrono::Utc::now().timestamp(),
        receiver: Address::new(&addr, ServiceFlags::NONE),
        sender: Address::new(
            &SocketAddr::new("0.0.0.0".parse().unwrap(), 0),
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

async fn scrape_and_update_nodes() {
    let seeds: Vec<&'static str> = vec![
        "dnsseed.bluematt.me",
        "dnsseed.bitcoin.dashjr.org",
        "seed.bitcoinstats.com",
        "seed.btc.petertodd.org",
        "seed.bitcoin.sprovoost.nl",
        "dnsseed.emzy.de",
        "seed.bitcoin.wiz.biz",
    ];
    let mut seed_addrs = Vec::new();
    for seed in &seeds {
        if let Ok(addrs) = lookup_host(format!("{}:8333", seed)).await {
            seed_addrs.extend(addrs.into_iter());
        }
    }
    let mut libre = std::collections::HashSet::<SocketAddr>::new();
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
    let mut cache = NODE_CACHE.write().await;
    cache.nodes = peers;
    cache.last_refresh = Some(Instant::now());
}

// --- Main Entrypoint ---
#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt().with_target(false).init();

    // --- Config ---
    let cfg = Arc::new(AppConfig::parse());

    // --- Node cache refresh (background) ---
    tokio::spawn(async {
        loop {
            scrape_and_update_nodes().await;
            // Refresh every 10 minutes
            tokio::time::sleep(Duration::from_secs(600)).await;
        }
    });

    // --- Axum app ---
    let app = Router::new()
        .route("/", get(index))
        .route("/submit", post(submit_tx))
        .with_state(cfg);

    let addr = SocketAddr::from(([0, 0, 0, 0], 8080));
    tracing::info!("listening on {}", addr);

    axum::serve(
        TcpListener::bind(addr).await?,
        app.into_make_service_with_connect_info::<SocketAddr>(),
    )
    .await?;

    Ok(())
}
