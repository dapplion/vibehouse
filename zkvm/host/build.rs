//! Build script that compiles the SP1 guest program ELF.
//!
//! This uses `sp1-build` to cross-compile the guest program for the
//! riscv32im-succinct-zkvm-elf target. The resulting ELF binary is
//! embedded into the host binary at compile time via `include_bytes!`.
//!
//! Requires: SP1 toolchain installed (`sp1up && sp1up`)

fn main() {
    sp1_build::build_program("../guest");
}
