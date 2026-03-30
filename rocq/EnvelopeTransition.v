(** * Execution Payload Envelope State Transition

    Models process_execution_payload_envelope from Gloas and proves:
    - Availability index bounds (slot % SLOTS_PER_HISTORICAL_ROOT < SLOTS_PER_HISTORICAL_ROOT)
    - Payment index bounds in envelope context (reuses BuilderPayments result)
    - State mutation frame: exactly which fields change
    - Payment blanking: payment entry is zeroed after processing
    - State header caching idempotence: non-default state_root is preserved
    - Verification completeness: all envelope fields are checked

    Rust source:
    - [envelope_processing.rs:201-382] process_execution_payload_envelope

    Spec reference: consensus-specs/specs/gloas/beacon-chain.md#new-process_execution_payload
*)

From Stdlib Require Import Arith.
From Stdlib Require Import Lia.
From Stdlib Require Import List.
From Stdlib Require Import Bool.
Import ListNotations.

(** ** Constants *)

Parameter SLOTS_PER_EPOCH : nat.
Parameter SLOTS_PER_HISTORICAL_ROOT : nat.
Axiom slots_per_epoch_pos : SLOTS_PER_EPOCH > 0.
Axiom slots_per_historical_root_pos : SLOTS_PER_HISTORICAL_ROOT > 0.

Definition PAYMENTS_LEN := 2 * SLOTS_PER_EPOCH.

(** ** Hash model *)

(** We represent hashes as natural numbers. Hash256::default() is 0. *)
Definition Hash := nat.
Definition default_hash : Hash := 0.

(** ** Execution block hash *)
Definition ExecBlockHash := nat.

(** ** State model *)

(** Simplified envelope-relevant beacon state. We model only the fields
    that process_execution_payload_envelope reads or writes. *)
Record EnvelopeState := mkEnvelopeState {
  st_slot : nat;
  st_header_state_root : Hash;           (** latest_block_header.state_root *)
  st_header_root : Hash;                 (** tree_hash_root of latest_block_header *)
  st_latest_block_hash : ExecBlockHash;  (** latest execution block hash *)
  st_pending_payments : list nat;        (** builder_pending_payments (amounts) *)
  st_pending_withdrawals : list nat;     (** builder_pending_withdrawals (amounts) *)
  st_availability : list bool;           (** execution_payload_availability bitvector *)
  st_bid_builder_index : nat;            (** committed bid fields *)
  st_bid_prev_randao : Hash;
  st_bid_gas_limit : nat;
  st_bid_block_hash : ExecBlockHash;
  st_genesis_time : nat;
}.

(** ** Envelope model *)

Record Envelope := mkEnvelope {
  env_beacon_block_root : Hash;
  env_slot : nat;
  env_builder_index : nat;
  env_state_root : Hash;
  env_payload_prev_randao : Hash;
  env_payload_gas_limit : nat;
  env_payload_block_hash : ExecBlockHash;
  env_payload_parent_hash : ExecBlockHash;
  env_payload_timestamp : nat;
}.

(** ** Timestamp computation *)

Parameter SECONDS_PER_SLOT : nat.
Axiom seconds_per_slot_pos : SECONDS_PER_SLOT > 0.

Definition compute_timestamp (genesis_time slot : nat) : nat :=
  genesis_time + slot * SECONDS_PER_SLOT.

(** ** Availability index computation *)

Definition availability_index (slot : nat) : nat :=
  slot mod SLOTS_PER_HISTORICAL_ROOT.

(** ** Payment index computation (same as BuilderPayments.bid_slot_index) *)

Definition payment_index (slot : nat) : nat :=
  SLOTS_PER_EPOCH + (slot mod SLOTS_PER_EPOCH).

(** ** Availability index bounds *)

Theorem availability_index_valid : forall slot,
  availability_index slot < SLOTS_PER_HISTORICAL_ROOT.
Proof.
  intros. unfold availability_index.
  pose proof slots_per_historical_root_pos.
  apply Nat.mod_upper_bound. lia.
Qed.

(** The availability index is always non-negative (trivially true for nat). *)
Theorem availability_index_lower_bound : forall slot,
  availability_index slot >= 0.
Proof.
  intros. unfold availability_index. lia.
Qed.

(** ** Payment index bounds in envelope context *)

Theorem payment_index_lower_bound : forall slot,
  payment_index slot >= SLOTS_PER_EPOCH.
Proof.
  intros. unfold payment_index. lia.
Qed.

Theorem payment_index_upper_bound : forall slot,
  payment_index slot < PAYMENTS_LEN.
Proof.
  intros. unfold payment_index, PAYMENTS_LEN.
  pose proof slots_per_epoch_pos.
  pose proof (Nat.mod_upper_bound slot SLOTS_PER_EPOCH).
  lia.
Qed.

(** ** Envelope verification predicate *)

(** All consistency checks that process_execution_payload_envelope performs
    before any state mutation. If this predicate holds, the function proceeds
    to mutate state. *)
Definition envelope_consistent (st : EnvelopeState) (env : Envelope) : Prop :=
  (* beacon block root matches *)
  env_beacon_block_root env = st_header_root st /\
  (* slot matches *)
  env_slot env = st_slot st /\
  (* builder index matches committed bid *)
  env_builder_index env = st_bid_builder_index st /\
  (* prev_randao matches committed bid *)
  env_payload_prev_randao env = st_bid_prev_randao st /\
  (* gas limit matches committed bid *)
  env_payload_gas_limit env = st_bid_gas_limit st /\
  (* block hash matches committed bid *)
  env_payload_block_hash env = st_bid_block_hash st /\
  (* parent hash matches latest execution block hash *)
  env_payload_parent_hash env = st_latest_block_hash st /\
  (* timestamp matches computed *)
  env_payload_timestamp env = compute_timestamp (st_genesis_time st) (st_slot st).

(** Verification completeness: the predicate checks exactly the 8 fields
    that the spec requires. We prove that violating any single check causes
    the predicate to be false. *)

Theorem verify_block_root_required : forall st env,
  env_beacon_block_root env <> st_header_root st ->
  ~ envelope_consistent st env.
Proof.
  intros st env Hneq [H _]. contradiction.
Qed.

Theorem verify_slot_required : forall st env,
  env_slot env <> st_slot st ->
  ~ envelope_consistent st env.
Proof.
  intros st env Hneq [_ [H _]]. contradiction.
Qed.

Theorem verify_builder_index_required : forall st env,
  env_builder_index env <> st_bid_builder_index st ->
  ~ envelope_consistent st env.
Proof.
  intros st env Hneq [_ [_ [H _]]]. contradiction.
Qed.

Theorem verify_prev_randao_required : forall st env,
  env_payload_prev_randao env <> st_bid_prev_randao st ->
  ~ envelope_consistent st env.
Proof.
  intros st env Hneq [_ [_ [_ [H _]]]]. contradiction.
Qed.

Theorem verify_gas_limit_required : forall st env,
  env_payload_gas_limit env <> st_bid_gas_limit st ->
  ~ envelope_consistent st env.
Proof.
  intros st env Hneq [_ [_ [_ [_ [H _]]]]]. contradiction.
Qed.

Theorem verify_block_hash_required : forall st env,
  env_payload_block_hash env <> st_bid_block_hash st ->
  ~ envelope_consistent st env.
Proof.
  intros st env Hneq [_ [_ [_ [_ [_ [H _]]]]]]. contradiction.
Qed.

Theorem verify_parent_hash_required : forall st env,
  env_payload_parent_hash env <> st_latest_block_hash st ->
  ~ envelope_consistent st env.
Proof.
  intros st env Hneq [_ [_ [_ [_ [_ [_ [H _]]]]]]]. contradiction.
Qed.

Theorem verify_timestamp_required : forall st env,
  env_payload_timestamp env <> compute_timestamp (st_genesis_time st) (st_slot st) ->
  ~ envelope_consistent st env.
Proof.
  intros st env Hneq [_ [_ [_ [_ [_ [_ [_ H]]]]]]]. contradiction.
Qed.

(** ** State header caching idempotence *)

(** If the header state root is already non-default, the caching step is a no-op.
    This models lines 240-243: only fills when state_root == Hash256::default(). *)
Definition cache_header_state_root (header_state_root parent_root : Hash) : Hash :=
  if Nat.eqb header_state_root default_hash then parent_root
  else header_state_root.

Theorem cache_idempotent_when_set : forall sr parent,
  sr <> default_hash ->
  cache_header_state_root sr parent = sr.
Proof.
  intros sr parent Hneq.
  unfold cache_header_state_root, default_hash in *.
  destruct (Nat.eqb sr 0) eqn:E.
  - apply Nat.eqb_eq in E. lia.
  - reflexivity.
Qed.

Theorem cache_fills_when_default : forall parent,
  cache_header_state_root default_hash parent = parent.
Proof.
  intros. unfold cache_header_state_root, default_hash. simpl. reflexivity.
Qed.

(** ** List update helper *)

Fixpoint list_update {A : Type} (l : list A) (i : nat) (v : A) : list A :=
  match l, i with
  | [], _ => []
  | _ :: t, 0 => v :: t
  | h :: t, S n => h :: list_update t n v
  end.

Fixpoint list_set_bool (l : list bool) (i : nat) (v : bool) : list bool :=
  match l, i with
  | [], _ => []
  | _ :: t, 0 => v :: t
  | h :: t, S n => h :: list_set_bool t n v
  end.

Lemma list_update_length : forall {A : Type} (l : list A) i v,
  length (list_update l i v) = length l.
Proof.
  intros A l. induction l; intros; destruct i; simpl; auto.
Qed.

Lemma list_update_same : forall {A : Type} (l : list A) i v,
  i < length l ->
  nth_error (list_update l i v) i = Some v.
Proof.
  intros A l. induction l; intros; destruct i; simpl in *; try lia.
  - reflexivity.
  - apply IHl. lia.
Qed.

Lemma list_update_other : forall {A : Type} (l : list A) i j v,
  i <> j ->
  nth_error (list_update l i v) j = nth_error l j.
Proof.
  intros A l. induction l; intros; destruct i; destruct j; simpl in *; try lia; auto.
Qed.

(** ** State transition model *)

(** Models the state mutations in process_execution_payload_envelope after
    all verification checks pass. This is the "effect" of lines 336-367. *)
Definition apply_envelope (st : EnvelopeState) (env : Envelope) : option EnvelopeState :=
  let pidx := payment_index (st_slot st) in
  let aidx := availability_index (st_slot st) in
  (* Check payment index is valid *)
  match nth_error (st_pending_payments st) pidx with
  | None => None
  | Some payment_amount =>
    (* Check availability index is valid *)
    if Nat.ltb aidx (length (st_availability st)) then
      let new_payments := list_update (st_pending_payments st) pidx 0 in
      let new_withdrawals :=
        if Nat.ltb 0 payment_amount
        then st_pending_withdrawals st ++ [payment_amount]
        else st_pending_withdrawals st in
      let new_availability := list_set_bool (st_availability st) aidx true in
      let new_header_sr := cache_header_state_root (st_header_state_root st) default_hash in
      Some (mkEnvelopeState
        (st_slot st)
        new_header_sr
        (st_header_root st)
        (env_payload_block_hash env)     (* latest_block_hash updated *)
        new_payments                      (* payment entry blanked *)
        new_withdrawals                   (* possibly appended *)
        new_availability                  (* bit set *)
        (st_bid_builder_index st)
        (st_bid_prev_randao st)
        (st_bid_gas_limit st)
        (st_bid_block_hash st)
        (st_genesis_time st))
    else None
  end.

(** ** Well-formedness *)

Definition state_well_formed (st : EnvelopeState) : Prop :=
  length (st_pending_payments st) = PAYMENTS_LEN /\
  length (st_availability st) >= SLOTS_PER_HISTORICAL_ROOT.

(** ** Transition always succeeds on well-formed state *)

Theorem apply_envelope_succeeds : forall st env,
  state_well_formed st ->
  exists st', apply_envelope st env = Some st'.
Proof.
  intros st env [Hpay Havail].
  unfold apply_envelope.
  (* Payment index is valid *)
  assert (Hpidx : payment_index (st_slot st) < PAYMENTS_LEN)
    by apply payment_index_upper_bound.
  assert (Hpidx_valid : payment_index (st_slot st) < length (st_pending_payments st))
    by lia.
  destruct (nth_error (st_pending_payments st) (payment_index (st_slot st))) eqn:Epay.
  2: { apply nth_error_None in Epay. lia. }
  (* Availability index is valid *)
  assert (Haidx : availability_index (st_slot st) < SLOTS_PER_HISTORICAL_ROOT)
    by apply availability_index_valid.
  assert (Haidx_valid : availability_index (st_slot st) < length (st_availability st))
    by lia.
  assert (Nat.ltb (availability_index (st_slot st)) (length (st_availability st)) = true) as Eav.
  { apply Nat.ltb_lt. lia. }
  rewrite Eav.
  eexists. reflexivity.
Qed.

(** ** Payment blanking correctness *)

(** After apply_envelope, the payment at payment_index is 0. *)
Theorem payment_blanked : forall st env st',
  state_well_formed st ->
  apply_envelope st env = Some st' ->
  nth_error (st_pending_payments st') (payment_index (st_slot st)) = Some 0.
Proof.
  intros st env st' Hwf Happly.
  unfold apply_envelope in Happly.
  destruct (nth_error (st_pending_payments st) (payment_index (st_slot st))) eqn:Epay;
    [| discriminate].
  destruct (Nat.ltb _ _) eqn:Eav; [| discriminate].
  injection Happly as <-. simpl.
  apply list_update_same.
  apply nth_error_Some. rewrite Epay. discriminate.
Qed.

(** Other payment entries are unchanged. *)
Theorem payment_other_preserved : forall st env st' j,
  state_well_formed st ->
  apply_envelope st env = Some st' ->
  j <> payment_index (st_slot st) ->
  nth_error (st_pending_payments st') j = nth_error (st_pending_payments st) j.
Proof.
  intros st env st' j Hwf Happly Hneq.
  unfold apply_envelope in Happly.
  destruct (nth_error (st_pending_payments st) (payment_index (st_slot st))) eqn:Epay;
    [| discriminate].
  destruct (Nat.ltb _ _) eqn:Eav; [| discriminate].
  injection Happly as <-. simpl.
  apply list_update_other. auto.
Qed.

(** ** Latest block hash updated *)

Theorem block_hash_updated : forall st env st',
  apply_envelope st env = Some st' ->
  st_latest_block_hash st' = env_payload_block_hash env.
Proof.
  intros st env st' Happly.
  unfold apply_envelope in Happly.
  destruct (nth_error _ _); [| discriminate].
  destruct (Nat.ltb _ _); [| discriminate].
  injection Happly as <-. reflexivity.
Qed.

(** ** Withdrawal queueing correctness *)

(** If payment amount > 0, it is appended to withdrawals. *)
Theorem withdrawal_queued_when_positive : forall st env st' amount,
  state_well_formed st ->
  apply_envelope st env = Some st' ->
  nth_error (st_pending_payments st) (payment_index (st_slot st)) = Some amount ->
  amount > 0 ->
  st_pending_withdrawals st' = st_pending_withdrawals st ++ [amount].
Proof.
  intros st env st' amount Hwf Happly Hpay Hpos.
  unfold apply_envelope in Happly.
  rewrite Hpay in Happly.
  destruct (Nat.ltb _ _) eqn:Eav; [| discriminate].
  injection Happly as <-. simpl.
  assert (Nat.ltb 0 amount = true) as Hlt.
  { apply Nat.ltb_lt. lia. }
  rewrite Hlt. reflexivity.
Qed.

(** If payment amount = 0, withdrawals are unchanged. *)
Theorem withdrawal_unchanged_when_zero : forall st env st',
  state_well_formed st ->
  apply_envelope st env = Some st' ->
  nth_error (st_pending_payments st) (payment_index (st_slot st)) = Some 0 ->
  st_pending_withdrawals st' = st_pending_withdrawals st.
Proof.
  intros st env st' Hwf Happly Hpay.
  unfold apply_envelope in Happly.
  rewrite Hpay in Happly.
  destruct (Nat.ltb _ _) eqn:Eav; [| discriminate].
  injection Happly as <-. simpl. reflexivity.
Qed.

(** ** Slot is preserved *)

Theorem slot_preserved : forall st env st',
  apply_envelope st env = Some st' ->
  st_slot st' = st_slot st.
Proof.
  intros st env st' Happly.
  unfold apply_envelope in Happly.
  destruct (nth_error _ _); [| discriminate].
  destruct (Nat.ltb _ _); [| discriminate].
  injection Happly as <-. reflexivity.
Qed.

(** ** Genesis time is preserved *)

Theorem genesis_time_preserved : forall st env st',
  apply_envelope st env = Some st' ->
  st_genesis_time st' = st_genesis_time st.
Proof.
  intros st env st' Happly.
  unfold apply_envelope in Happly.
  destruct (nth_error _ _); [| discriminate].
  destruct (Nat.ltb _ _); [| discriminate].
  injection Happly as <-. reflexivity.
Qed.

(** ** Payments length preserved *)

Theorem payments_length_preserved : forall st env st',
  state_well_formed st ->
  apply_envelope st env = Some st' ->
  length (st_pending_payments st') = length (st_pending_payments st).
Proof.
  intros st env st' Hwf Happly.
  unfold apply_envelope in Happly.
  destruct (nth_error _ _) eqn:Epay; [| discriminate].
  destruct (Nat.ltb _ _); [| discriminate].
  injection Happly as <-. simpl.
  apply list_update_length.
Qed.

(** ** Availability index uniqueness across slots *)

(** Two different slots (within the same historical root window) map to
    different availability indices. This ensures no availability collision. *)
Theorem availability_index_injective : forall s1 s2,
  s1 mod SLOTS_PER_HISTORICAL_ROOT <> s2 mod SLOTS_PER_HISTORICAL_ROOT ->
  availability_index s1 <> availability_index s2.
Proof.
  intros s1 s2 Hmod.
  unfold availability_index. exact Hmod.
Qed.

(** Within a single SLOTS_PER_HISTORICAL_ROOT window, the index is injective. *)
Theorem availability_index_injective_in_window : forall s1 s2,
  s1 / SLOTS_PER_HISTORICAL_ROOT = s2 / SLOTS_PER_HISTORICAL_ROOT ->
  availability_index s1 = availability_index s2 ->
  s1 = s2.
Proof.
  intros s1 s2 Hwin Hidx.
  unfold availability_index in Hidx.
  pose proof slots_per_historical_root_pos.
  rewrite (Nat.div_mod_eq s1 SLOTS_PER_HISTORICAL_ROOT).
  rewrite (Nat.div_mod_eq s2 SLOTS_PER_HISTORICAL_ROOT).
  lia.
Qed.

(** ** Frame theorem: bid fields unchanged *)

(** The committed bid fields are never modified by envelope processing.
    This is important because the bid was set during block processing (phase 1)
    and must not be corrupted by envelope processing (phase 2). *)
Theorem bid_fields_preserved : forall st env st',
  apply_envelope st env = Some st' ->
  st_bid_builder_index st' = st_bid_builder_index st /\
  st_bid_prev_randao st' = st_bid_prev_randao st /\
  st_bid_gas_limit st' = st_bid_gas_limit st /\
  st_bid_block_hash st' = st_bid_block_hash st.
Proof.
  intros st env st' Happly.
  unfold apply_envelope in Happly.
  destruct (nth_error _ _); [| discriminate].
  destruct (Nat.ltb _ _); [| discriminate].
  injection Happly as <-. simpl.
  repeat split; reflexivity.
Qed.
