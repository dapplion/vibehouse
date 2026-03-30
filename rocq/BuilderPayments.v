(** * Builder Payment Arithmetic

    Models the builder pending payments slot indexing and epoch rotation from Gloas.

    Rust sources:
    - [gloas.rs:172-175] Slot indexing: SLOTS_PER_EPOCH + (bid.slot % SLOTS_PER_EPOCH)
    - [per_epoch_processing/gloas.rs:14-78] Payment sweep and rotation

    Spec reference: consensus-specs/specs/gloas/beacon-chain.md
      process_execution_payload_bid: state.builder_pending_payments[SLOTS_PER_EPOCH + bid.slot % SLOTS_PER_EPOCH]
      process_builder_pending_payments: rotate first half out, move second half to first, clear second half
*)

From Stdlib Require Import Arith.
From Stdlib Require Import Lia.
From Stdlib Require Import List.
Import ListNotations.

(** ** Constants *)

Parameter SLOTS_PER_EPOCH : nat.
Axiom slots_per_epoch_pos : SLOTS_PER_EPOCH > 0.

(** The builder_pending_payments vector has 2 * SLOTS_PER_EPOCH entries.
    First half: previous epoch's slots (indices 0..SLOTS_PER_EPOCH-1).
    Second half: current epoch's slots (indices SLOTS_PER_EPOCH..2*SLOTS_PER_EPOCH-1). *)
Definition PAYMENTS_LEN := 2 * SLOTS_PER_EPOCH.

(** ** Slot index computation *)

(** The index into builder_pending_payments for a bid at a given slot.
    Spec: SLOTS_PER_EPOCH + (slot % SLOTS_PER_EPOCH)
    This always maps to the second half of the array (current epoch). *)
Definition bid_slot_index (slot : nat) : nat :=
  SLOTS_PER_EPOCH + (slot mod SLOTS_PER_EPOCH).

(** ** Slot index bounds *)

(** The slot index is always in the second half: [SLOTS_PER_EPOCH, 2*SLOTS_PER_EPOCH). *)
Theorem bid_slot_index_lower_bound : forall slot,
  bid_slot_index slot >= SLOTS_PER_EPOCH.
Proof.
  intros. unfold bid_slot_index. lia.
Qed.

Theorem bid_slot_index_upper_bound : forall slot,
  bid_slot_index slot < PAYMENTS_LEN.
Proof.
  intros. unfold bid_slot_index, PAYMENTS_LEN.
  pose proof slots_per_epoch_pos.
  pose proof (Nat.mod_upper_bound slot SLOTS_PER_EPOCH).
  lia.
Qed.

(** Combined: slot index is always a valid index into the payments vector. *)
Theorem bid_slot_index_valid : forall slot,
  bid_slot_index slot < PAYMENTS_LEN.
Proof. exact bid_slot_index_upper_bound. Qed.

(** Slot index never collides with the first half (previous epoch). *)
Theorem bid_slot_index_not_in_first_half : forall slot,
  bid_slot_index slot >= SLOTS_PER_EPOCH.
Proof. exact bid_slot_index_lower_bound. Qed.

(** ** Slot index uniqueness within an epoch *)

(** Two slots in the same epoch map to different indices.
    This ensures no payment collision within a single epoch. *)
Theorem bid_slot_index_injective_within_epoch : forall s1 s2,
  s1 / SLOTS_PER_EPOCH = s2 / SLOTS_PER_EPOCH ->
  bid_slot_index s1 = bid_slot_index s2 ->
  s1 = s2.
Proof.
  intros s1 s2 Hepoch Hidx.
  unfold bid_slot_index in Hidx.
  assert (s1 mod SLOTS_PER_EPOCH = s2 mod SLOTS_PER_EPOCH) as Hmod by lia.
  pose proof slots_per_epoch_pos.
  rewrite (Nat.div_mod_eq s1 SLOTS_PER_EPOCH).
  rewrite (Nat.div_mod_eq s2 SLOTS_PER_EPOCH).
  lia.
Qed.

(** ** Epoch rotation model *)

(** A payment vector is a list of natural numbers (weights). *)
Definition payments := list nat.

(** Rotation: move second half to first, zero out second half.
    Models process_builder_pending_payments rotation step. *)
Definition rotate (ps : payments) : payments :=
  firstn SLOTS_PER_EPOCH (skipn SLOTS_PER_EPOCH ps) ++ repeat 0 SLOTS_PER_EPOCH.

(** ** Rotation properties *)

(** After rotation, the result has the correct length. *)
Lemma rotate_length : forall ps,
  length ps = PAYMENTS_LEN ->
  length (rotate ps) = PAYMENTS_LEN.
Proof.
  intros ps Hlen.
  unfold rotate, PAYMENTS_LEN in *.
  rewrite app_length, repeat_length, firstn_length, skipn_length.
  pose proof slots_per_epoch_pos.
  lia.
Qed.

(** After rotation, the second half is all zeros — ready for new bids. *)
Lemma rotate_second_half_zero : forall ps i,
  length ps = PAYMENTS_LEN ->
  SLOTS_PER_EPOCH <= i ->
  i < PAYMENTS_LEN ->
  nth i (rotate ps) 0 = 0.
Proof.
  intros ps i Hlen Hlo Hhi.
  unfold rotate, PAYMENTS_LEN in *.
  pose proof slots_per_epoch_pos.
  rewrite app_nth2.
  - rewrite firstn_length, skipn_length.
    assert (Nat.min SLOTS_PER_EPOCH (length ps - SLOTS_PER_EPOCH) = SLOTS_PER_EPOCH) as Hmin
      by (apply Nat.min_l; lia).
    rewrite Hmin.
    apply nth_repeat; lia.
  - rewrite firstn_length, skipn_length.
    assert (Nat.min SLOTS_PER_EPOCH (length ps - SLOTS_PER_EPOCH) = SLOTS_PER_EPOCH) as Hmin
      by (apply Nat.min_l; lia).
    lia.
Qed.

(** After rotation, the first half contains what was the second half.
    This is the key correctness property: bids from the current epoch
    become the "previous epoch" entries for the next epoch's sweep. *)
Lemma rotate_first_half_preserved : forall ps i,
  length ps = PAYMENTS_LEN ->
  i < SLOTS_PER_EPOCH ->
  nth i (rotate ps) 0 = nth (SLOTS_PER_EPOCH + i) ps 0.
Proof.
  intros ps i Hlen Hlt.
  unfold rotate, PAYMENTS_LEN in *.
  pose proof slots_per_epoch_pos.
  rewrite app_nth1.
  - rewrite nth_firstn.
    assert (i <? SLOTS_PER_EPOCH = true) as Hcmp by (apply Nat.ltb_lt; lia).
    rewrite Hcmp.
    rewrite nth_skipn. reflexivity.
  - rewrite firstn_length, skipn_length.
    assert (Nat.min SLOTS_PER_EPOCH (length ps - SLOTS_PER_EPOCH) = SLOTS_PER_EPOCH) as Hmin
      by (apply Nat.min_l; lia).
    lia.
Qed.

(** ** Quorum threshold for payments *)

Parameter BUILDER_PAYMENT_THRESHOLD_NUMERATOR : nat.
Parameter BUILDER_PAYMENT_THRESHOLD_DENOMINATOR : nat.
Axiom threshold_denom_pos : BUILDER_PAYMENT_THRESHOLD_DENOMINATOR > 0.

(** Quorum threshold: (total_active_balance / SLOTS_PER_EPOCH) * NUM / DENOM *)
Definition payment_quorum (total_active_balance : nat) : nat :=
  (total_active_balance / SLOTS_PER_EPOCH) *
  BUILDER_PAYMENT_THRESHOLD_NUMERATOR /
  BUILDER_PAYMENT_THRESHOLD_DENOMINATOR.

(** A payment qualifies if its weight meets the quorum. *)
Definition qualifies (weight : nat) (total_active_balance : nat) : Prop :=
  weight >= payment_quorum total_active_balance.

(** ** Payment sweep correctness *)

(** The sweep checks only the first SLOTS_PER_EPOCH entries (previous epoch). *)
Definition sweep_qualifying (ps : payments) (total_active_balance : nat) : list nat :=
  filter (fun w => payment_quorum total_active_balance <=? w) (firstn SLOTS_PER_EPOCH ps).

(** Sweep never touches the second half (current epoch bids). *)
Theorem sweep_ignores_current_epoch : forall ps total_active_balance,
  length ps = PAYMENTS_LEN ->
  sweep_qualifying ps total_active_balance =
  sweep_qualifying (firstn SLOTS_PER_EPOCH ps ++ repeat 0 SLOTS_PER_EPOCH) total_active_balance.
Proof.
  intros ps tab Hlen.
  unfold sweep_qualifying, PAYMENTS_LEN in *.
  pose proof slots_per_epoch_pos.
  (* Key insight: firstn SLOTS_PER_EPOCH (firstn SLOTS_PER_EPOCH ps ++ _)
     = firstn SLOTS_PER_EPOCH ps, because firstn SLOTS_PER_EPOCH ps has
     length SLOTS_PER_EPOCH when length ps >= SLOTS_PER_EPOCH. *)
  assert (Hfn : firstn SLOTS_PER_EPOCH (firstn SLOTS_PER_EPOCH ps ++ repeat 0 SLOTS_PER_EPOCH)
              = firstn SLOTS_PER_EPOCH ps).
  { rewrite firstn_app, firstn_length.
    assert (Nat.min SLOTS_PER_EPOCH (length ps) = SLOTS_PER_EPOCH) as Hmin
      by (apply Nat.min_l; lia).
    rewrite Hmin.
    replace (SLOTS_PER_EPOCH - SLOTS_PER_EPOCH) with 0 by lia.
    rewrite firstn_O, app_nil_r.
    rewrite firstn_firstn.
    replace (Nat.min SLOTS_PER_EPOCH SLOTS_PER_EPOCH) with SLOTS_PER_EPOCH by lia.
    reflexivity. }
  rewrite Hfn. reflexivity.
Qed.

(** After rotation + sweep, old current-epoch bids become the sweep targets.
    This is the central correctness theorem: it proves the rotation faithfully
    moves current-epoch bids into the sweep window for the next epoch. *)
Theorem rotate_then_sweep_targets_old_bids : forall ps total_active_balance,
  length ps = PAYMENTS_LEN ->
  sweep_qualifying (rotate ps) total_active_balance =
  filter (fun w => payment_quorum total_active_balance <=? w)
    (firstn SLOTS_PER_EPOCH (skipn SLOTS_PER_EPOCH ps)).
Proof.
  intros ps tab Hlen.
  unfold sweep_qualifying, rotate, PAYMENTS_LEN in *.
  pose proof slots_per_epoch_pos.
  assert (Hfn : firstn SLOTS_PER_EPOCH
    (firstn SLOTS_PER_EPOCH (skipn SLOTS_PER_EPOCH ps) ++ repeat 0 SLOTS_PER_EPOCH)
    = firstn SLOTS_PER_EPOCH (skipn SLOTS_PER_EPOCH ps)).
  { rewrite firstn_app, firstn_length, skipn_length.
    assert (Nat.min SLOTS_PER_EPOCH (length ps - SLOTS_PER_EPOCH) = SLOTS_PER_EPOCH) as Hmin
      by (apply Nat.min_l; lia).
    rewrite Hmin.
    replace (SLOTS_PER_EPOCH - SLOTS_PER_EPOCH) with 0 by lia.
    rewrite firstn_O, app_nil_r.
    rewrite firstn_firstn.
    replace (Nat.min SLOTS_PER_EPOCH SLOTS_PER_EPOCH) with SLOTS_PER_EPOCH by lia.
    reflexivity. }
  rewrite Hfn. reflexivity.
Qed.
