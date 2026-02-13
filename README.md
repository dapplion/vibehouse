# VIBEHOUSE

### the lighthouse fork that runs on vibes

```
 _    _____ ____  ______  ______  __  _______ ______
| |  / /  _/ __ )/ ____/ / / / / / / / / ___// ____/
| | / // // __  / __/   / /_/ / / / / /\__ \/ __/
| |/ // // /_/ / /___  / __  / /_/ / /___/ / /___
|___/___/_____/_____/ /_/ /_/\____/\____/____/_____/
```

> Forked from [Lighthouse v8.0.1](https://github.com/sigp/lighthouse) (post-Fulu). From here on out, it's just vibes.

---

**vibehouse** is a community-driven Ethereum consensus client. Same rock-solid Lighthouse core, but we ship faster, break things sometimes, and accept feature requests from literally anyone.

We don't have a roadmap. We have a **vibemap**. See [plan.md](./plan.md).

## What makes vibehouse different

- **Vibes-first development** - If the community wants it, we build it. Open an issue. Dare us.
- **Gloas on day one** - Tracking [ethereum/consensus-specs](https://github.com/ethereum/consensus-specs) gloas fork as top priority. ePBS or bust.
- **Spec test maximalism** - Running the latest consensus spec tests. All of them. Always.
- **Coverage obsession** - We track test coverage and it only goes up.
- **Kurtosis in CI** - Multi-client testnets on every PR. If it doesn't survive kurtosis, it doesn't merge.
- **Apache 2.0** - Same license, same freedom. Built in Rust.

## Quick start

```bash
# clone it
git clone https://github.com/dapplion/vibehouse.git
cd vibehouse

# build it
make

# vibe with it
./target/release/lighthouse --help
```

## Upstream

vibehouse tracks [sigp/lighthouse](https://github.com/sigp/lighthouse) as upstream. We regularly cherry-pick and merge fixes. The fork point is **v8.0.1** (Fulu mainnet).

## Contributing

Open a PR. Open an issue. Drop a meme. We're here for it.

See [plan.md](./plan.md) for what we're working on and where help is needed.

## Documentation

For general Lighthouse usage, the [Lighthouse Book](https://lighthouse-book.sigmaprime.io) still applies.

## Contact

- GitHub Issues: [dapplion/vibehouse](https://github.com/dapplion/vibehouse/issues)

## License

Apache 2.0, same as upstream Lighthouse.
