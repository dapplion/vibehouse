# Pre-built Binaries

Each Vibehouse release contains several downloadable binaries in the "Assets"
section of the release. You can find the [releases
on Github](https://github.com/dapplion/vibehouse/releases).

## Platforms

Binaries are supplied for the following platforms:

- `x86_64-unknown-linux-gnu`: AMD/Intel 64-bit processors (most desktops, laptops, servers)
- `aarch64-unknown-linux-gnu`: 64-bit ARM processors (Raspberry Pi 4)
- `aarch64-apple-darwin`: macOS with ARM chips
- `x86_64-windows`: Windows with 64-bit processors

## Usage

Each binary is contained in a `.tar.gz` archive. For this example, lets assume the user needs
a `x86_64` binary.

### Steps

1. Go to the [Releases](https://github.com/dapplion/vibehouse/releases) page and
   select the latest release.
1. Download the `vibehouse-${VERSION}-x86_64-unknown-linux-gnu.tar.gz` binary. For example, to obtain the binary file for v4.0.1 (the latest version at the time of writing), a user can run the following commands in a linux terminal:

    ```bash
    cd ~
    curl -LO https://github.com/dapplion/vibehouse/releases/download/v4.0.1/vibehouse-v4.0.1-x86_64-unknown-linux-gnu.tar.gz
    tar -xvf vibehouse-v4.0.1-x86_64-unknown-linux-gnu.tar.gz
    ```

1. Test the binary with `./vibehouse --version` (it should print the version).
1. (Optional) Move the `vibehouse` binary to a location in your `PATH`, so the `vibehouse` command can be called from anywhere. For example, to copy `vibehouse` from the current directory to `usr/bin`, run `sudo cp vibehouse /usr/bin`.

> Windows users will need to execute the commands in Step 2 from PowerShell.
