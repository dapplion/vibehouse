# workstream: kurtosis multi-client testing

> status: **not started** | priority: 4

## overview

Set up multi-client interop testing using Kurtosis ethereum-package. Ensure vibehouse can run alongside other CL and EL clients on a testnet.

## dependencies

- Docker image for vibehouse
- Kurtosis CLI
- ethereum-package fork with vibehouse support

## sources

- Kurtosis: https://github.com/kurtosis-tech/kurtosis
- ethereum-package: https://github.com/ethpandaops/ethereum-package

## next steps

1. Understand the existing lighthouse docker build process
2. Test building a vibehouse docker image
3. Fork ethereum-package and add vibehouse support
4. Run first local testnet

## log

- 2026-02-13: workstream created
