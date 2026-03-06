// BeaconChainError closures trigger result_large_err (128+ bytes).
#![allow(clippy::result_large_err)]

mod service;

pub use service::SlasherService;
