(** * PTC Quorum Arithmetic

    Models the Payload Timeliness Committee (PTC) quorum logic from Gloas fork choice.

    Rust source: [proto_array_fork_choice.rs:1267-1269]
      let ptc_quorum_threshold = spec.ptc_size / 2;
      n.ptc_weight > ptc_quorum_threshold

    Spec reference: consensus-specs/specs/gloas/fork-choice.md
      PAYLOAD_TIMELY_THRESHOLD = PTC_SIZE // 2
      has_payload_quorum: sum(store.payload_timeliness_votes[root]) > PAYLOAD_TIMELY_THRESHOLD
*)

From Stdlib Require Import Arith.
From Stdlib Require Import Lia.
From Stdlib Require Import List.
Import ListNotations.

(** ** Constants *)

(** PTC size — mainnet = 512, minimal = 2. We prove for arbitrary PTC_SIZE > 0. *)
Parameter PTC_SIZE : nat.
Axiom ptc_size_pos : PTC_SIZE > 0.

Definition PAYLOAD_TIMELY_THRESHOLD := PTC_SIZE / 2.

(** ** Vote model *)

(** A vote is either timely (1) or not (0). *)
Definition vote := nat.
Definition timely : vote := 1.
Definition not_timely : vote := 0.

(** Sum of votes from PTC members. *)
Fixpoint vote_sum (votes : list vote) : nat :=
  match votes with
  | [] => 0
  | v :: vs => v + vote_sum vs
  end.

(** ** Quorum predicate *)

(** Spec: has_payload_quorum iff sum > threshold (strictly greater). *)
Definition has_quorum (votes : list vote) : Prop :=
  vote_sum votes > PAYLOAD_TIMELY_THRESHOLD.

(** ** Helper lemmas *)

Lemma vote_sum_le_length : forall (votes : list vote),
  Forall (fun v => v <= 1) votes ->
  vote_sum votes <= length votes.
Proof.
  intros votes H.
  induction votes as [| v vs IHvs].
  - simpl. lia.
  - simpl. inversion H; subst.
    specialize (IHvs H3). lia.
Qed.

(** Count of timely votes. *)
Definition count_timely (votes : list vote) : nat :=
  length (filter (fun v => Nat.eqb v 1) votes).

Lemma vote_sum_eq_count_timely : forall (votes : list vote),
  Forall (fun v => v <= 1) votes ->
  vote_sum votes = count_timely votes.
Proof.
  intros votes H.
  induction votes as [| v vs IHvs].
  - reflexivity.
  - simpl. inversion H; subst.
    specialize (IHvs H3).
    unfold count_timely in *. simpl.
    destruct (Nat.eqb v 1) eqn:Heq.
    + apply Nat.eqb_eq in Heq. subst. simpl. lia.
    + apply Nat.eqb_neq in Heq. assert (v = 0) by lia. subst. simpl. lia.
Qed.

(** ** Soundness: quorum requires strict majority *)

Theorem quorum_requires_majority : forall (votes : list vote),
  Forall (fun v => v <= 1) votes ->
  length votes = PTC_SIZE ->
  has_quorum votes ->
  count_timely votes > PTC_SIZE / 2.
Proof.
  intros votes Hbounded Hlen Hquorum.
  unfold has_quorum, PAYLOAD_TIMELY_THRESHOLD in Hquorum.
  rewrite vote_sum_eq_count_timely in Hquorum by assumption.
  exact Hquorum.
Qed.

(** ** Completeness: if strict majority votes timely, quorum holds *)

Theorem majority_gives_quorum : forall (votes : list vote),
  Forall (fun v => v <= 1) votes ->
  length votes = PTC_SIZE ->
  count_timely votes > PTC_SIZE / 2 ->
  has_quorum votes.
Proof.
  intros votes Hbounded Hlen Hmaj.
  unfold has_quorum, PAYLOAD_TIMELY_THRESHOLD.
  rewrite vote_sum_eq_count_timely by assumption.
  exact Hmaj.
Qed.

(** ** Threshold arithmetic properties *)

Lemma threshold_is_half : PAYLOAD_TIMELY_THRESHOLD = PTC_SIZE / 2.
Proof. reflexivity. Qed.

(** The threshold is strictly less than PTC_SIZE when PTC_SIZE > 0. *)
Lemma threshold_lt_ptc_size : PAYLOAD_TIMELY_THRESHOLD < PTC_SIZE.
Proof.
  unfold PAYLOAD_TIMELY_THRESHOLD.
  pose proof ptc_size_pos.
  apply Nat.div_lt_upper_bound; lia.
Qed.

(** ** Zero and unanimous vote properties *)

Lemma vote_sum_repeat_zero : forall n, vote_sum (repeat not_timely n) = 0.
Proof.
  intros n. induction n as [| k IHk].
  - reflexivity.
  - simpl. unfold not_timely in *. simpl. exact IHk.
Qed.

Lemma vote_sum_repeat_one : forall n, vote_sum (repeat timely n) = n.
Proof.
  intros n. induction n as [| k IHk].
  - reflexivity.
  - simpl. unfold timely in *. lia.
Qed.

(** No quorum is possible with all-zero votes. *)
Theorem no_quorum_with_zero_votes :
  forall n, ~ has_quorum (repeat not_timely n).
Proof.
  intros n H.
  unfold has_quorum, PAYLOAD_TIMELY_THRESHOLD in H.
  rewrite vote_sum_repeat_zero in H. lia.
Qed.

(** Unanimous quorum always holds when committee size > threshold.
    Since PTC_SIZE > PTC_SIZE/2 for PTC_SIZE > 0, unanimous votes always reach quorum. *)
Theorem unanimous_quorum_general : forall n,
  n > PTC_SIZE / 2 ->
  has_quorum (repeat timely n).
Proof.
  intros n Hgt.
  unfold has_quorum, PAYLOAD_TIMELY_THRESHOLD.
  rewrite vote_sum_repeat_one. exact Hgt.
Qed.

(** For the full PTC, unanimous vote always reaches quorum. *)
Corollary unanimous_quorum_full_ptc :
  has_quorum (repeat timely PTC_SIZE).
Proof.
  apply unanimous_quorum_general.
  pose proof ptc_size_pos.
  apply Nat.div_lt_upper_bound; lia.
Qed.

(** ** Monotonicity: adding a timely vote cannot break quorum *)

Lemma vote_sum_cons : forall v vs, vote_sum (v :: vs) = v + vote_sum vs.
Proof. reflexivity. Qed.

Theorem quorum_preserved_by_timely_vote : forall votes,
  has_quorum votes ->
  has_quorum (timely :: votes).
Proof.
  intros votes H.
  unfold has_quorum in *.
  unfold PAYLOAD_TIMELY_THRESHOLD in *.
  rewrite vote_sum_cons. unfold timely.
  lia.
Qed.
