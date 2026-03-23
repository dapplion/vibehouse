.PHONY: tests

EF_TESTS = "testing/ef_tests"
STATE_TRANSITION_VECTORS = "testing/state_transition_vectors"
EXECUTION_ENGINE_INTEGRATION = "testing/execution_engine_integration"
GIT_TAG := $(shell git describe --tags --candidates 1)
BIN_DIR = "bin"

X86_64_TAG = "x86_64-unknown-linux-gnu"
BUILD_PATH_X86_64 = "target/$(X86_64_TAG)/release"
AARCH64_TAG = "aarch64-unknown-linux-gnu"
BUILD_PATH_AARCH64 = "target/$(AARCH64_TAG)/release"
RISCV64_TAG = "riscv64gc-unknown-linux-gnu"
BUILD_PATH_RISCV64 = "target/$(RISCV64_TAG)/release"

PINNED_NIGHTLY ?= nightly

# List of features to use when cross-compiling. Can be overridden via the environment.
CROSS_FEATURES ?= gnosis,slasher-lmdb,slasher-mdbx,slasher-redb

# Cargo profile for Cross builds. Default is for local builds, CI uses an override.
CROSS_PROFILE ?= release

# List of features to use when running EF tests.
EF_TEST_FEATURES ?=

# List of features to use when running CI tests.
TEST_FEATURES ?=

# Cargo profile for regular builds.
PROFILE ?= release

# List of all hard forks. This list is used to set env variables for several tests so that
# they run for different forks.
FORKS=phase0 altair bellatrix capella deneb electra fulu gloas

# List of all recent hard forks. This list is used to set env variables for http_api tests
RECENT_FORKS=electra fulu gloas

# Extra flags for Cargo
CARGO_INSTALL_EXTRA_FLAGS?=

# Builds the vibehouse binary in release (optimized).
#
# Binaries will most likely be found in `./target/release`
install:
	cargo install --path vibehouse --force --locked \
		--features "$(FEATURES)" \
		--profile "$(PROFILE)" \
		$(CARGO_INSTALL_EXTRA_FLAGS)

# Builds the lcli binary in release (optimized).
install-lcli:
	cargo install --path lcli --force --locked \
		--features "$(FEATURES)" \
		--profile "$(PROFILE)" \
		$(CARGO_INSTALL_EXTRA_FLAGS)

# The following commands use `cross` to build a cross-compile.
#
# These commands require that:
#
# - `cross` is installed (`cargo install cross`).
# - Docker is running.
# - The current user is in the `docker` group.
#
# The resulting binaries will be created in the `target/` directory.
build-x86_64:
	cross build --bin vibehouse --target x86_64-unknown-linux-gnu --features "portable,$(CROSS_FEATURES)" --profile "$(CROSS_PROFILE)" --locked
build-aarch64:
	# JEMALLOC_SYS_WITH_LG_PAGE=16 tells jemalloc to support up to 64-KiB
	# pages, which are commonly used by aarch64 systems.
	# See: https://github.com/sigp/lighthouse/issues/5244
	JEMALLOC_SYS_WITH_LG_PAGE=16 cross build --bin vibehouse --target aarch64-unknown-linux-gnu --features "portable,$(CROSS_FEATURES)" --profile "$(CROSS_PROFILE)" --locked
build-riscv64:
	cross build --bin vibehouse --target riscv64gc-unknown-linux-gnu --features "portable,$(CROSS_FEATURES)" --profile "$(CROSS_PROFILE)" --locked

build-lcli-x86_64:
	cross build --bin lcli --target x86_64-unknown-linux-gnu --features "portable" --profile "$(CROSS_PROFILE)" --locked
build-lcli-aarch64:
	# JEMALLOC_SYS_WITH_LG_PAGE=16 tells jemalloc to support up to 64-KiB
	# pages, which are commonly used by aarch64 systems.
	# See: https://github.com/sigp/lighthouse/issues/5244
	JEMALLOC_SYS_WITH_LG_PAGE=16 cross build --bin lcli --target aarch64-unknown-linux-gnu --features "portable" --profile "$(CROSS_PROFILE)" --locked
build-lcli-riscv64:
	cross build --bin lcli --target riscv64gc-unknown-linux-gnu --features "portable" --profile "$(CROSS_PROFILE)" --locked

# extracts the current source date for reproducible builds
SOURCE_DATE := $(shell git log -1 --pretty=%ct)

# Default image for x86_64
RUST_IMAGE_AMD64 ?= rust:1.88-bullseye@sha256:8e3c421122bf4cd3b2a866af41a4dd52d87ad9e315fd2cb5100e87a7187a9816

# Reproducible build for x86_64
build-reproducible-x86_64:
	DOCKER_BUILDKIT=1 docker build \
		--build-arg RUST_TARGET="x86_64-unknown-linux-gnu" \
		--build-arg RUST_IMAGE=$(RUST_IMAGE_AMD64) \
		--build-arg SOURCE_DATE=$(SOURCE_DATE) \
		-f Dockerfile.reproducible \
		-t vibehouse:reproducible-amd64 .

# Default image for arm64
RUST_IMAGE_ARM64 ?= rust:1.88-bullseye@sha256:8b22455a7ce2adb1355067638284ee99d21cc516fab63a96c4514beaf370aa94

# Reproducible build for aarch64
build-reproducible-aarch64:
	DOCKER_BUILDKIT=1 docker build \
		--platform linux/arm64 \
		--build-arg RUST_TARGET="aarch64-unknown-linux-gnu" \
		--build-arg RUST_IMAGE=$(RUST_IMAGE_ARM64) \
		--build-arg SOURCE_DATE=$(SOURCE_DATE) \
		-f Dockerfile.reproducible \
		-t vibehouse:reproducible-arm64 .

# Build both architectures
build-reproducible-all: build-reproducible-x86_64 build-reproducible-aarch64

# Create a `.tar.gz` containing a binary for a specific target.
define tarball_release_binary
	cp $(1)/vibehouse $(BIN_DIR)/vibehouse
	cd $(BIN_DIR) && \
		tar -czf vibehouse-$(GIT_TAG)-$(2)$(3).tar.gz vibehouse && \
		rm vibehouse
endef

# Create a series of `.tar.gz` files in the BIN_DIR directory, each containing
# a `vibehouse` binary for a different target.
#
# The current git tag will be used as the version in the output file names. You
# will likely need to use `git tag` and create a semver tag (e.g., `v0.2.3`).
build-release-tarballs:
	[ -d $(BIN_DIR) ] || mkdir -p $(BIN_DIR)
	$(MAKE) build-x86_64
	$(call tarball_release_binary,$(BUILD_PATH_X86_64),$(X86_64_TAG),"")
	$(MAKE) build-aarch64
	$(call tarball_release_binary,$(BUILD_PATH_AARCH64),$(AARCH64_TAG),"")
	$(MAKE) build-riscv64
	$(call tarball_release_binary,$(BUILD_PATH_RISCV64),$(RISCV64_TAG),"")



# Runs the full workspace tests in **release**, without downloading any additional
# test vectors.
test-release:
	cargo nextest run --workspace --release --features "$(TEST_FEATURES)" \
		--exclude ef_tests --exclude beacon_chain --exclude slasher --exclude network \
		--exclude http_api


# Runs the full workspace tests in **debug**, without downloading any additional test
# vectors.
test-debug:
	cargo nextest run --workspace --features "$(TEST_FEATURES)" \
		--exclude ef_tests --exclude beacon_chain --exclude network --exclude http_api

# Runs cargo-fmt (linter).
cargo-fmt:
	cargo fmt --all -- --check

# Typechecks benchmark code
check-benches:
	cargo check --workspace --benches --features "$(TEST_FEATURES)"


# Runs EF test vectors
run-ef-tests:
	rm -rf $(EF_TESTS)/.accessed_file_log.txt
	cargo nextest run --release -p ef_tests --features "ef_tests,$(EF_TEST_FEATURES)"
	cargo nextest run --release -p ef_tests --features "ef_tests,$(EF_TEST_FEATURES),fake_crypto"
	./$(EF_TESTS)/check_all_files_accessed.py $(EF_TESTS)/.accessed_file_log.txt $(EF_TESTS)/consensus-spec-tests

# Run the tests in the `beacon_chain` crate for all known forks.
test-beacon-chain: $(patsubst %,test-beacon-chain-%,$(FORKS))

test-beacon-chain-%:
	env FORK_NAME=$* cargo nextest run --release --features "fork_from_env,slasher/lmdb,$(TEST_FEATURES)" -p beacon_chain

# Run the tests in the `http_api` crate for recent forks.
test-http-api: $(patsubst %,test-http-api-%,$(RECENT_FORKS))

test-http-api-%:
	env FORK_NAME=$* cargo nextest run --release --features "beacon_chain/fork_from_env" -p http_api


# Run the tests in the `operation_pool` crate for all known forks.
test-op-pool: $(patsubst %,test-op-pool-%,$(FORKS))

test-op-pool-%:
	env FORK_NAME=$* cargo nextest run --release \
		--features "beacon_chain/fork_from_env,$(TEST_FEATURES)"\
		-p operation_pool

# Run the tests in the `network` crate for all known forks.
test-network: $(patsubst %,test-network-%,$(FORKS))

test-network-%:
	env FORK_NAME=$* cargo nextest run --release \
		--features "fork_from_env,$(TEST_FEATURES)" \
		-p network

# Run the tests in the `slasher` crate for all supported database backends.
test-slasher:
	cargo nextest run --release -p slasher --features "lmdb,$(TEST_FEATURES)"
	cargo nextest run --release -p slasher --no-default-features --features "redb,$(TEST_FEATURES)"
	cargo nextest run --release -p slasher --no-default-features --features "mdbx,$(TEST_FEATURES)"
	cargo nextest run --release -p slasher --features "lmdb,mdbx,redb,$(TEST_FEATURES)" # all backends enabled

# Runs only the tests/state_transition_vectors tests.
run-state-transition-tests:
	make -C $(STATE_TRANSITION_VECTORS) test

# Downloads and runs the EF test vectors.
test-ef: make-ef-tests run-ef-tests

# Downloads and runs the nightly EF test vectors.
test-ef-nightly: make-ef-tests-nightly run-ef-tests

# Runs tests checking interop between Vibehouse and execution clients.
test-exec-engine:
	make -C $(EXECUTION_ENGINE_INTEGRATION) test

# Runs the full workspace tests in release, without downloading any additional
# test vectors.
test: test-release

# Updates the CLI help text pages in the book, building with Docker (primarily for Windows users).
cli:
	docker run --rm --user=root \
	-v ${PWD}:/home/runner/actions-runner/vibehouse sigmaprime/github-runner \
	bash -c 'cd vibehouse && make && ./scripts/cli.sh'

# Updates the CLI help text pages in the book, building using local `cargo`.
cli-local:
	make && ./scripts/cli.sh

# Check for markdown files
mdlint:
	./scripts/mdlint.sh

# Runs the entire test suite, downloading test vectors if required.
test-full: cargo-fmt test-release test-debug test-ef test-exec-engine

# Lints the code for bad style and potentially unsafe arithmetic using Clippy.
# Runs clippy with workspace-wide lints enforced as errors.
lint:
	cargo clippy --workspace --benches --tests $(EXTRA_CLIPPY_OPTS) --features "$(TEST_FEATURES)" -- \
		-D clippy::fn_to_numeric_cast_any \
		-D clippy::manual_let_else \
		-D clippy::large_stack_frames \
		-D clippy::disallowed_methods \
		-D clippy::derive_partial_eq_without_eq \
		-D clippy::redundant_closure_for_method_calls \
		-D clippy::cloned_instead_of_copied \
		-D clippy::flat_map_option \
		-D clippy::from_iter_instead_of_collect \
		-D clippy::semicolon_if_nothing_returned \
		-D clippy::inconsistent_struct_constructor \
		-D clippy::needless_for_each \
		-D clippy::implicit_clone \
		-D clippy::range_plus_one \
		-D clippy::checked_conversions \
		-D clippy::if_not_else \
		-D clippy::redundant_else \
		-D clippy::inefficient_to_string \
		-D clippy::items_after_statements \
		-D clippy::trivially_copy_pass_by_ref \
		-D clippy::unused_self \
		-D clippy::map_unwrap_or \
		-D clippy::match_same_arms \
		-D clippy::single_match_else \
		-D clippy::unnested_or_patterns \
		-D clippy::explicit_into_iter_loop \
		-D clippy::explicit_iter_loop \
		-D clippy::manual_string_new \
		-D clippy::uninlined_format_args \
		-D clippy::needless_raw_string_hashes \
		-D clippy::default_trait_access \
		-D clippy::redundant_closure \
		-D clippy::ptr_as_ptr \
		-D clippy::macro_use_imports \
		-D clippy::needless_continue \
		-D clippy::map_flatten \
		-D clippy::manual_assert \
		-D clippy::ref_option_ref \
		-D clippy::option_option \
		-D clippy::verbose_bit_mask \
		-D clippy::zero_sized_map_values \
		-D clippy::stable_sort_primitive \
		-D clippy::string_add_assign \
		-D clippy::naive_bytecount \
		-D clippy::filter_map_next \
		-D clippy::mut_mut \
		-D clippy::suspicious_operation_groupings \
		-D clippy::literal_string_with_formatting_args \
		-D clippy::unnecessary_struct_initialization \
		-D clippy::string_lit_as_bytes \
		-D clippy::suboptimal_flops \
		-D clippy::branches_sharing_code \
		-D clippy::unused_async \
		-D clippy::same_functions_in_if_condition \
		-D clippy::no_effect_underscore_binding \
		-D clippy::manual_is_variant_and \
		-D clippy::bool_to_int_with_if \
		-D clippy::cast_lossless \
		-D clippy::manual_ok_or \
		-D clippy::manual_instant_elapsed \
		-D clippy::unicode_not_nfc \
		-D clippy::transmute_ptr_to_ptr \
		-D clippy::ref_as_ptr \
		-D clippy::explicit_deref_methods \
		-D clippy::invalid_upcast_comparisons \
		-D clippy::large_types_passed_by_value \
		-D clippy::manual_find_map \
		-D clippy::mismatching_type_param_order \
		-D clippy::collection_is_never_read \
		-D clippy::debug_assert_with_mut_call \
		-D clippy::empty_line_after_doc_comments \
		-D clippy::empty_line_after_outer_attr \
		-D clippy::format_push_string \
		-D clippy::imprecise_flops \
		-D clippy::index_refutable_slice \
		-D clippy::iter_not_returning_iterator \
		-D clippy::iter_on_empty_collections \
		-D clippy::iter_on_single_items \
		-D clippy::large_digit_groups \
		-D clippy::large_include_file \
		-D clippy::lossy_float_literal \
		-D clippy::manual_clamp \
		-D clippy::manual_filter_map \
		-D clippy::manual_is_ascii_check \
		-D clippy::manual_is_power_of_two \
		-D clippy::map_identity \
		-D clippy::match_wildcard_for_single_variants \
		-D clippy::maybe_infinite_iter \
		-D clippy::mixed_read_write_in_expression \
		-D clippy::needless_bitwise_bool \
		-D clippy::neg_multiply \
		-D clippy::no_mangle_with_rust_abi \
		-D clippy::path_buf_push_overwrite \
		-D clippy::range_minus_one \
		-D clippy::readonly_write_lock \
		-D clippy::redundant_feature_names \
		-D clippy::rest_pat_in_fully_bound_structs \
		-D clippy::single_char_pattern \
		-D clippy::suspicious_xor_used_as_pow \
		-D clippy::transmute_undefined_repr \
		-D clippy::tuple_array_conversions \
		-D clippy::type_id_on_box \
		-D clippy::unnecessary_join \
		-D clippy::unnecessary_safety_doc \
		-D clippy::unreadable_literal \
		-D clippy::unused_peekable \
		-D clippy::unused_rounding \
		-D clippy::enum_glob_use \
		-D clippy::ignored_unit_patterns \
		-D clippy::borrow_as_ptr \
		-D clippy::case_sensitive_file_extension_comparisons \
		-D clippy::comparison_chain \
		-D clippy::elidable_lifetime_names \
		-D clippy::inline_always \
		-D clippy::into_iter_without_iter \
		-D clippy::manual_ilog2 \
		-D clippy::missing_fields_in_debug \
		-D clippy::assigning_clones \
		-D clippy::should_panic_without_expect \
		-D clippy::ignore_without_reason \
		-D clippy::ref_binding_to_reference \
		-D clippy::fn_params_excessive_bools \
		-D clippy::decimal_bitwise_operands \
		-D clippy::needless_pass_by_ref_mut \
		-D clippy::unnecessary_wraps \
		-D clippy::manual_flatten \
		-D clippy::map_entry \
		-D clippy::unnecessary_lazy_evaluations \
		-D clippy::or_fun_call \
		-D clippy::manual_strip \
		-D clippy::match_bool \
		-D clippy::search_is_some \
		-D clippy::len_zero \
		-D clippy::redundant_guards \
		-D clippy::manual_map \
		-D clippy::useless_vec \
		-D clippy::option_as_ref_cloned \
		-D clippy::redundant_type_annotations \
		-D clippy::copy_iterator \
		-D clippy::nonstandard_macro_braces \
		-D clippy::zero_prefixed_literal \
		-D clippy::iter_filter_is_some \
		-D clippy::iter_filter_is_ok \
		-D clippy::empty_enum_variants_with_brackets \
		-D clippy::needless_lifetimes \
		-D clippy::needless_return \
		-D clippy::needless_borrow \
		-D clippy::needless_borrows_for_generic_args \
		-D clippy::needless_range_loop \
		-D clippy::manual_range_contains \
		-D clippy::single_component_path_imports \
		-D clippy::unnecessary_to_owned \
		-D clippy::ptr_arg \
		-D clippy::clone_on_copy \
		-D clippy::unnecessary_cast \
		-D clippy::map_clone \
		-D clippy::if_same_then_else \
		-D clippy::neg_cmp_op_on_partial_ord \
		-D clippy::no_effect \
		-D clippy::unnecessary_operation \
		-D clippy::identity_op \
		-D clippy::double_parens \
		-D clippy::let_and_return \
		-D clippy::match_single_binding \
		-D clippy::wildcard_in_or_patterns \
		-D clippy::match_wild_err_arm \
		-D clippy::verbose_file_reads \
		-D clippy::from_over_into \
		-D clippy::flat_map_identity \
		-D clippy::iter_with_drain \
		-D clippy::unused_io_amount \
		-D clippy::equatable_if_let \
		-D clippy::rc_buffer \
		-D clippy::rc_mutex \
		-D clippy::string_add \
		-D clippy::implicit_hasher \
		-D clippy::manual_c_str_literals \
		-D clippy::unnecessary_fallible_conversions \
		-D clippy::implied_bounds_in_impls \
		-D clippy::no_effect_replace \
		-D clippy::legacy_numeric_constants \
		-D clippy::manual_pattern_char_comparison \
		-D clippy::single_char_add_str \
		-D clippy::iter_kv_map \
		-D clippy::collapsible_str_replace \
		-D clippy::used_underscore_items \
		-D clippy::while_let_on_iterator \
		-D clippy::unnecessary_filter_map \
		-D clippy::manual_next_back \
		-D clippy::cloned_ref_to_slice_refs \
		-D clippy::unchecked_time_subtraction \
		-D clippy::trivial_regex \
		-D clippy::useless_let_if_seq \
		-D warnings

# Lints the code using Clippy and automatically fix some simple compiler warnings.
lint-fix:
	EXTRA_CLIPPY_OPTS="--fix --allow-staged --allow-dirty" $(MAKE) lint-full

# Also run the lints on the optimized-only tests
lint-full:
	RUSTFLAGS="-C debug-assertions=no -W unreachable_pub $(RUSTFLAGS)" $(MAKE) lint

# Runs the makefile in the `ef_tests` repo.
#
# May download and extract an archive of test vectors from the ethereum
# repositories. At the time of writing, this was several hundred MB of
# downloads which extracts into several GB of test vectors.
make-ef-tests:
	make -C $(EF_TESTS)

# Download/extract the nightly EF test vectors.
make-ef-tests-nightly:
	CONSENSUS_SPECS_TEST_VERSION=nightly make -C $(EF_TESTS)

# Verifies that crates compile with fuzzing features enabled
arbitrary-fuzz:
	cargo check -p state_processing --features arbitrary-fuzz,$(TEST_FEATURES)
	cargo check -p slashing_protection --features arbitrary-fuzz,$(TEST_FEATURES)

# Runs cargo audit (Audit Cargo.lock files for crates with security vulnerabilities reported to the RustSec Advisory Database)
audit: install-audit audit-CI

install-audit:
	cargo install --force cargo-audit

audit-CI:
	cargo audit

# Runs `cargo vendor` to make sure dependencies can be vendored for packaging, reproducibility and archival purpose.
vendor:
	cargo vendor

# Runs `cargo udeps` to check for unused dependencies
udeps:
	cargo +$(PINNED_NIGHTLY) udeps --tests --all-targets --release --features "$(TEST_FEATURES)"

# Performs a `cargo` clean and cleans the `ef_tests` directory.
clean:
	cargo clean
	make -C $(EF_TESTS) clean
	make -C $(STATE_TRANSITION_VECTORS) clean
