# tx-pigeon 🕊️

Send a little pigeon out to all the **libre-relay**  nodes and poop on them with whatever transaction you like. 

in the name of censorship resistance money 

## Setup

```bash
# 1. Install the Rust toolchain (if you haven’t already)
curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh

# 2. Clone & build the project
git clone https://github.com/stutxo/tx-pigeon.git
cd tx-pigeon

# 3. Fire away 🕊️💩
cargo run -- --tx \
  020000000001019d8c84a78cb5e032c20ce46868a64c0a2f88090f790ab57320b53804484c7a31 \
  0000000000fdffffff026517000000000000225120ead5bf5032f0564c3f3689d1057c4895311390d11500b94510f5a0fb9f2ba98 \
  7000000000000000fd94016a4d9001f09f92a9f09f92a9f09f92a9f09f92a9f09f92a9f09f92a9f09f92a9f09f92a9f09f92a9f09f92a9f09 \
  f92a9f09f92a9f09f92a9f09f92a9f09f92a9f09f92a9f09f92a9f09f92a9f09f92a9f09f92a9f09f92a9f09f92a9f09f92a9f09f92a9f09 \
  f92a9f09f92a90140f38ed8b347a8ec4a5a32ea85d9283a54dc0d18d3298cd1ddb98c6f2ceef24ed6f6b19fab372aafc30185941ac5558f262fe13b3680ff01df3ebe7f37ab0f3de700000000
```

Heres one i relayed earlier: 

https://mempool.space/tx/449653d50f4a9fad160e4eb10c3c4e6d3013a516e871187df6f1017fda58abe4

![alt text](image.png)