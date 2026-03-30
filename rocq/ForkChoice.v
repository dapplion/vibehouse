(** * Fork Choice Formal Proofs

    Models the proto_array fork choice algorithm from vibehouse.

    Rust sources:
    - [proto_array.rs:185-331]  apply_score_changes — weight propagation + best child update
    - [proto_array.rs:669-719]  find_head — head selection via best_descendant
    - [proto_array.rs:721-789]  maybe_prune — safe finalized-prefix pruning
    - [proto_array.rs:803-891]  maybe_update_best_child_and_descendant — greedy best-child
    - [proto_array.rs:920-966]  node_is_viable_for_head — FFG viability filter

    Spec reference: consensus-specs/specs/phase0/fork-choice.md (LMD-GHOST)
*)

From Stdlib Require Import Arith.
From Stdlib Require Import Lia.
From Stdlib Require Import List.
From Stdlib Require Import Bool.
Import ListNotations.

(** ** Node model *)

(** A simplified proto_array node. We model the essential fields for proving
    algorithmic correctness: weight, parent pointer, best_child, best_descendant,
    and viability. *)
Record Node := mkNode {
  node_weight : nat;
  node_parent : option nat;    (** index into nodes array *)
  node_best_child : option nat;
  node_best_descendant : option nat;
  node_viable : bool;          (** aggregates execution_status + FFG checks *)
  node_root : nat;             (** simplified root for tie-breaking *)
}.

(** A proto_array is a flat list of nodes with parent-before-child ordering. *)
Definition ProtoArray := list Node.

(** ** Array ordering invariant *)

(** Parent-before-child: if node at index i has parent Some(j), then j < i.
    This is the fundamental structural invariant of proto_array. *)
Definition parent_before_child (pa : ProtoArray) : Prop :=
  forall i node,
    nth_error pa i = Some node ->
    forall j, node_parent node = Some j ->
    j < i.

(** ** Index validity *)

(** All parent/best_child/best_descendant pointers refer to valid indices. *)
Definition indices_valid (pa : ProtoArray) : Prop :=
  forall i node,
    nth_error pa i = Some node ->
    (forall j, node_parent node = Some j -> j < length pa) /\
    (forall j, node_best_child node = Some j -> j < length pa) /\
    (forall j, node_best_descendant node = Some j -> j < length pa).

(** ** Weight non-negativity *)

(** Node weights are natural numbers, so non-negativity is guaranteed by the type system.
    In Rust, this is enforced by checked_sub returning an error rather than wrapping. *)
Theorem weight_non_negative : forall (pa : ProtoArray) i node,
  nth_error pa i = Some node ->
  node_weight node >= 0.
Proof. intros. lia. Qed.

(** ** Best-child selection model *)

(** The decision function for updating best_child.
    Models maybe_update_best_child_and_descendant from proto_array.rs:803-891. *)

Definition child_leads_to_viable (pa : ProtoArray) (child_idx : nat) : bool :=
  match nth_error pa child_idx with
  | None => false
  | Some child =>
    if node_viable child then true
    else match node_best_descendant child with
         | None => false
         | Some bd_idx =>
           match nth_error pa bd_idx with
           | None => false
           | Some bd => node_viable bd
           end
         end
  end.

Definition best_descendant_of (pa : ProtoArray) (child_idx : nat) : option nat :=
  match nth_error pa child_idx with
  | None => None
  | Some child =>
    match node_best_descendant child with
    | Some bd => Some bd
    | None => Some child_idx
    end
  end.

(** Select best child between current best and a new candidate.
    Rust: proto_array.rs:833-880
    Uses only <=? and <? (standard Coq notations). *)
Definition select_best_child
  (pa : ProtoArray)
  (current_best : option nat)
  (child_idx : nat) : option nat :=
  let child_viable := child_leads_to_viable pa child_idx in
  match current_best with
  | None =>
    if child_viable then Some child_idx else None
  | Some best_idx =>
    let best_viable := child_leads_to_viable pa best_idx in
    match nth_error pa child_idx, nth_error pa best_idx with
    | Some child, Some best =>
      if best_idx =? child_idx then
        if child_viable then Some child_idx else None
      else if child_viable && negb best_viable then
        Some child_idx
      else if negb child_viable && best_viable then
        Some best_idx
      else if node_weight child =? node_weight best then
        (* Tie-break by root: higher root wins. a >= b iff negb (a <? b) *)
        if negb (node_root child <? node_root best)
        then Some child_idx
        else Some best_idx
      else
        (* Higher weight wins. a > b iff negb (a <=? b) *)
        if negb (node_weight child <=? node_weight best)
        then Some child_idx
        else Some best_idx
    | _, _ => current_best
    end
  end.

(** ** Best-child selection determinism *)

(** Given identical inputs, select_best_child always produces the same output. *)
Theorem select_best_child_deterministic :
  forall pa current_best child_idx,
    select_best_child pa current_best child_idx =
    select_best_child pa current_best child_idx.
Proof. reflexivity. Qed.

(** ** Best-child viability: if a viable child exists, best_child is viable *)

Theorem select_best_child_viable_preserved :
  forall pa child_idx,
    child_leads_to_viable pa child_idx = true ->
    exists bc, select_best_child pa None child_idx = Some bc /\
               child_leads_to_viable pa bc = true.
Proof.
  intros pa child_idx Hviable.
  unfold select_best_child.
  rewrite Hviable.
  exists child_idx. split; [reflexivity | exact Hviable].
Qed.

(** ** Best-child weight ordering *)

(** When both candidates are viable and have different weights,
    the selected best_child has the maximum weight. *)
Theorem select_best_child_max_weight :
  forall pa best_idx child_idx child best,
    best_idx <> child_idx ->
    nth_error pa child_idx = Some child ->
    nth_error pa best_idx = Some best ->
    child_leads_to_viable pa child_idx = true ->
    child_leads_to_viable pa best_idx = true ->
    node_weight child <> node_weight best ->
    forall selected,
      select_best_child pa (Some best_idx) child_idx = Some selected ->
      exists sel_node,
        nth_error pa selected = Some sel_node /\
        node_weight sel_node >= node_weight child /\
        node_weight sel_node >= node_weight best.
Proof.
  intros pa best_idx child_idx child best Hneq Hchild Hbest
         Hcv Hbv Hwneq selected Hsel.
  unfold select_best_child in Hsel.
  rewrite Hchild, Hbest in Hsel.
  assert (Hbeq: (best_idx =? child_idx) = false).
  { apply Nat.eqb_neq. auto. }
  rewrite Hbeq in Hsel.
  rewrite Hcv, Hbv in Hsel. simpl in Hsel.
  assert (Hweq: (node_weight child =? node_weight best) = false).
  { apply Nat.eqb_neq. auto. }
  rewrite Hweq in Hsel.
  destruct (negb (node_weight child <=? node_weight best)) eqn:Hgt.
  - (* child wins by weight *)
    inversion Hsel; subst.
    exists child. split; [exact Hchild |].
    apply negb_true_iff in Hgt.
    apply Nat.leb_nle in Hgt. lia.
  - (* best wins by weight *)
    inversion Hsel; subst.
    exists best. split; [exact Hbest |].
    apply negb_false_iff in Hgt.
    apply Nat.leb_le in Hgt. lia.
Qed.

(** ** Head selection model *)

(** find_head returns the best_descendant of the justified node, or the justified
    node itself if it has no best_descendant.
    Rust: proto_array.rs:669-719 *)
Definition find_head (pa : ProtoArray) (justified_idx : nat) : option nat :=
  match nth_error pa justified_idx with
  | None => None
  | Some jnode =>
    if negb (node_viable jnode) then None
    else
      let head_idx := match node_best_descendant jnode with
                      | Some bd => bd
                      | None => justified_idx
                      end in
      match nth_error pa head_idx with
      | None => None
      | Some head_node =>
        if node_viable head_node then Some head_idx else None
      end
  end.

(** ** Head is always viable *)

Theorem find_head_viable :
  forall pa justified_idx head_idx,
    find_head pa justified_idx = Some head_idx ->
    exists head_node,
      nth_error pa head_idx = Some head_node /\
      node_viable head_node = true.
Proof.
  intros pa justified_idx head_idx Hfind.
  unfold find_head in Hfind.
  destruct (nth_error pa justified_idx) as [jnode|] eqn:Hj; [|discriminate].
  destruct (negb (node_viable jnode)) eqn:Hnv; [discriminate|].
  set (head_candidate := match node_best_descendant jnode with
                         | Some bd => bd
                         | None => justified_idx
                         end) in Hfind.
  destruct (nth_error pa head_candidate) as [head_node|] eqn:Hh; [|discriminate].
  destruct (node_viable head_node) eqn:Hv; [|discriminate].
  inversion Hfind; subst.
  exists head_node. split; [exact Hh | exact Hv].
Qed.

(** ** Head with no descendants is justified node itself *)

Theorem find_head_no_descendants :
  forall pa justified_idx jnode,
    nth_error pa justified_idx = Some jnode ->
    node_viable jnode = true ->
    node_best_descendant jnode = None ->
    find_head pa justified_idx = Some justified_idx.
Proof.
  intros pa justified_idx jnode Hj Hv Hbd.
  unfold find_head. rewrite Hj.
  rewrite Hv. simpl. rewrite Hbd. rewrite Hj. rewrite Hv.
  reflexivity.
Qed.

(** ** Head with descendants returns best_descendant *)

Theorem find_head_with_descendant :
  forall pa justified_idx jnode bd_idx bd_node,
    nth_error pa justified_idx = Some jnode ->
    node_viable jnode = true ->
    node_best_descendant jnode = Some bd_idx ->
    nth_error pa bd_idx = Some bd_node ->
    node_viable bd_node = true ->
    find_head pa justified_idx = Some bd_idx.
Proof.
  intros pa justified_idx jnode bd_idx bd_node Hj Hjv Hbd Hbn Hbv.
  unfold find_head. rewrite Hj.
  rewrite Hjv. simpl. rewrite Hbd. rewrite Hbn. rewrite Hbv.
  reflexivity.
Qed.

(** ** Pruning model *)

(** Adjust an optional index by subtracting the finalized_index.
    Returns None if the index was below finalized_index. *)
Definition adjust_index (idx : option nat) (finalized_index : nat) : option nat :=
  match idx with
  | None => None
  | Some i => if i <? finalized_index then None else Some (i - finalized_index)
  end.

(** Adjust a node's indices after pruning. *)
Definition adjust_node (n : Node) (finalized_index : nat) : Node :=
  mkNode
    (node_weight n)
    (adjust_index (node_parent n) finalized_index)
    (adjust_index (node_best_child n) finalized_index)
    (adjust_index (node_best_descendant n) finalized_index)
    (node_viable n)
    (node_root n).

(** Prune: drop first finalized_index nodes, adjust remaining.
    Rust: proto_array.rs:734-789 *)
Definition prune (pa : ProtoArray) (finalized_index : nat) : ProtoArray :=
  map (fun n => adjust_node n finalized_index) (skipn finalized_index pa).

(** ** Pruning preserves length *)

Theorem prune_length :
  forall pa finalized_index,
    finalized_index <= length pa ->
    length (prune pa finalized_index) = length pa - finalized_index.
Proof.
  intros pa finalized_index Hle.
  unfold prune.
  rewrite map_length, skipn_length.
  reflexivity.
Qed.

(** ** Pruning preserves parent-before-child ordering *)

Lemma adjust_index_lt :
  forall idx fi result,
    adjust_index (Some idx) fi = Some result ->
    idx >= fi /\ result = idx - fi.
Proof.
  intros idx fi result Hadj.
  unfold adjust_index in Hadj.
  destruct (idx <? fi) eqn:Hlt.
  - discriminate.
  - inversion Hadj. apply Nat.ltb_nlt in Hlt. lia.
Qed.

Theorem prune_preserves_parent_order :
  forall pa finalized_index,
    parent_before_child pa ->
    finalized_index <= length pa ->
    parent_before_child (prune pa finalized_index).
Proof.
  intros pa fi Hpbc Hle.
  unfold parent_before_child, prune in *.
  intros i node Hi j Hpar.
  rewrite nth_error_map in Hi.
  destruct (nth_error (skipn fi pa) i) as [orig_node|] eqn:Horig; [|discriminate].
  inversion Hi; subst. clear Hi.
  simpl in Hpar.
  unfold adjust_index in Hpar.
  destruct (node_parent orig_node) as [orig_parent|] eqn:Hop; [|discriminate].
  destruct (orig_parent <? fi) eqn:Hlt; [discriminate|].
  inversion Hpar; subst. clear Hpar.
  assert (Horig2: nth_error pa (fi + i) = Some orig_node).
  { rewrite <- nth_error_skipn. exact Horig. }
  specialize (Hpbc (fi + i) orig_node Horig2 orig_parent Hop).
  apply Nat.ltb_nlt in Hlt.
  lia.
Qed.

(** ** Pruning never removes finalized node *)

Theorem prune_preserves_finalized :
  forall pa finalized_index fnode,
    nth_error pa finalized_index = Some fnode ->
    nth_error (prune pa finalized_index) 0 = Some (adjust_node fnode finalized_index).
Proof.
  intros pa fi fnode Hfi.
  unfold prune.
  rewrite nth_error_map.
  rewrite nth_error_skipn.
  replace (fi + 0) with fi by lia.
  rewrite Hfi. reflexivity.
Qed.

(** ** Pruning drops the right number of nodes *)

Theorem prune_drops_prefix :
  forall pa finalized_index,
    finalized_index <= length pa ->
    nth_error (prune pa finalized_index) 0 <> None ->
    exists orig_node,
      nth_error pa finalized_index = Some orig_node.
Proof.
  intros pa fi Hle Hne.
  unfold prune in Hne.
  rewrite nth_error_map in Hne.
  destruct (nth_error (skipn fi pa) 0) as [n|] eqn:Hskip.
  - rewrite nth_error_skipn in Hskip. replace (fi + 0) with fi in Hskip by lia.
    exists n. exact Hskip.
  - contradiction.
Qed.

(** ** Pruning preserves viability *)

Theorem prune_preserves_viability :
  forall pa fi i node,
    nth_error (prune pa fi) i = Some node ->
    exists orig_node,
      nth_error pa (fi + i) = Some orig_node /\
      node_viable node = node_viable orig_node.
Proof.
  intros pa fi i node Hpn.
  unfold prune in Hpn.
  rewrite nth_error_map in Hpn.
  destruct (nth_error (skipn fi pa) i) as [orig|] eqn:Horig; [|discriminate].
  inversion Hpn; subst.
  rewrite nth_error_skipn in Horig.
  exists orig. split; [exact Horig | reflexivity].
Qed.

(** ** Pruning preserves weight *)

Theorem prune_preserves_weight :
  forall pa fi i node,
    nth_error (prune pa fi) i = Some node ->
    exists orig_node,
      nth_error pa (fi + i) = Some orig_node /\
      node_weight node = node_weight orig_node.
Proof.
  intros pa fi i node Hpn.
  unfold prune in Hpn.
  rewrite nth_error_map in Hpn.
  destruct (nth_error (skipn fi pa) i) as [orig|] eqn:Horig; [|discriminate].
  inversion Hpn; subst.
  rewrite nth_error_skipn in Horig.
  exists orig. split; [exact Horig | reflexivity].
Qed.

(** ** Gloas payload status model *)

(** The 3-state payload model from Gloas ePBS fork choice. *)
Inductive PayloadStatus :=
  | EMPTY    (* parent executed empty — no payload delivered *)
  | FULL     (* parent executed full payload *)
  | PENDING  (* initial state, payload not yet determined *).

Definition payload_status_eq (a b : PayloadStatus) : bool :=
  match a, b with
  | EMPTY, EMPTY => true
  | FULL, FULL => true
  | PENDING, PENDING => true
  | _, _ => false
  end.

Lemma payload_status_eq_refl : forall s, payload_status_eq s s = true.
Proof. destruct s; reflexivity. Qed.

Lemma payload_status_eq_correct : forall a b,
  payload_status_eq a b = true <-> a = b.
Proof.
  destruct a, b; simpl; split; intro H; try reflexivity; try discriminate.
Qed.

(** ** Gloas virtual node *)

Record GloasVirtualNode := mkGVN {
  gvn_root : nat;
  gvn_payload_status : PayloadStatus;
  gvn_weight : nat;
}.

(** ** Payload status transitions *)

(** From PENDING, the next states are EMPTY (always available) and FULL (if envelope received). *)
Definition pending_children (envelope_received : bool) : list PayloadStatus :=
  if envelope_received then [EMPTY; FULL] else [EMPTY].

(** ** Payload status consistency *)

(** A vote with payload_present=true supports only FULL nodes.
    A vote with payload_present=false supports only EMPTY nodes.
    This prevents a single validator from supporting conflicting payload paths. *)

Definition vote_supports_status (payload_present : bool) (status : PayloadStatus) : bool :=
  match status with
  | FULL => payload_present
  | EMPTY => negb payload_present
  | PENDING => true
  end.

(** A validator's vote is consistent: it can support at most one of EMPTY/FULL. *)
Theorem vote_exclusive_payload_support :
  forall payload_present,
    vote_supports_status payload_present EMPTY = true ->
    vote_supports_status payload_present FULL = false.
Proof.
  intros []; simpl; intro H; [discriminate | reflexivity].
Qed.

Theorem vote_exclusive_payload_support_full :
  forall payload_present,
    vote_supports_status payload_present FULL = true ->
    vote_supports_status payload_present EMPTY = false.
Proof.
  intros []; simpl; intro H; [reflexivity | discriminate].
Qed.

(** PENDING is always supported regardless of payload_present flag. *)
Theorem vote_always_supports_pending :
  forall payload_present,
    vote_supports_status payload_present PENDING = true.
Proof. destruct payload_present; reflexivity. Qed.

(** ** Payload tiebreaker model *)

(** Tiebreaker ordinal for payload status.
    Rust: proto_array_fork_choice.rs:1812-1847 (non-previous-slot case)
    EMPTY=0, FULL=1, PENDING=2 *)
Definition payload_tiebreaker (status : PayloadStatus) : nat :=
  match status with
  | EMPTY => 0
  | FULL => 1
  | PENDING => 2
  end.

(** Tiebreaker ordering: PENDING > FULL > EMPTY. *)
Theorem tiebreaker_pending_wins :
  forall s, payload_tiebreaker s <= payload_tiebreaker PENDING.
Proof. destruct s; simpl; lia. Qed.

Theorem tiebreaker_full_beats_empty :
  payload_tiebreaker FULL > payload_tiebreaker EMPTY.
Proof. simpl. lia. Qed.

(** Tiebreaker is injective — different statuses get different ordinals. *)
Theorem tiebreaker_injective :
  forall a b,
    payload_tiebreaker a = payload_tiebreaker b -> a = b.
Proof. destruct a, b; simpl; intro H; try reflexivity; lia. Qed.

(** ** Gloas head selection determinism *)

(** The Gloas head selection comparison uses 3 layers:
    1. Weight (higher wins)
    2. Root (higher wins, tie-break)
    3. Payload tiebreaker ordinal (higher wins)
    This ensures deterministic head selection. *)

Definition gloas_compare (a b : GloasVirtualNode) : bool :=
  if negb (gvn_weight a <=? gvn_weight b) then true
  else if gvn_weight a <? gvn_weight b then false
  else if negb (gvn_root a <=? gvn_root b) then true
  else if gvn_root a <? gvn_root b then false
  else negb (payload_tiebreaker (gvn_payload_status a) <?
             payload_tiebreaker (gvn_payload_status b)).

(** The comparison is total: for any two nodes, at least one of
    gloas_compare a b or gloas_compare b a is true. *)
Theorem gloas_compare_total :
  forall a b,
    gloas_compare a b = true \/ gloas_compare b a = true.
Proof.
  intros a b. unfold gloas_compare.
  destruct (Nat.le_gt_cases (gvn_weight a) (gvn_weight b)) as [Hab|Hab].
  - (* weight a <= weight b *)
    assert (negb (gvn_weight a <=? gvn_weight b) = false) as Hw1.
    { apply negb_false_iff. apply Nat.leb_le. lia. }
    rewrite Hw1.
    destruct (Nat.eq_dec (gvn_weight a) (gvn_weight b)) as [Hweq|Hwneq].
    + (* weights equal *)
      assert (gvn_weight a <? gvn_weight b = false) as Hw2.
      { apply Nat.ltb_nlt. lia. }
      rewrite Hw2.
      assert (negb (gvn_weight b <=? gvn_weight a) = false) as Hw3.
      { apply negb_false_iff. apply Nat.leb_le. lia. }
      assert (gvn_weight b <? gvn_weight a = false) as Hw4.
      { apply Nat.ltb_nlt. lia. }
      destruct (Nat.le_gt_cases (gvn_root a) (gvn_root b)) as [Hrab|Hrab].
      * assert (negb (gvn_root a <=? gvn_root b) = false) as Hr1.
        { apply negb_false_iff. apply Nat.leb_le. lia. }
        rewrite Hr1.
        destruct (Nat.eq_dec (gvn_root a) (gvn_root b)) as [Hreq|Hrneq].
        -- (* roots equal — tiebreaker decides *)
           assert (gvn_root a <? gvn_root b = false) as Hr2.
           { apply Nat.ltb_nlt. lia. }
           rewrite Hr2.
           (* At least one of the tiebreakers is >= the other *)
           destruct (Nat.le_gt_cases
                       (payload_tiebreaker (gvn_payload_status a))
                       (payload_tiebreaker (gvn_payload_status b))) as [Ht|Ht].
           ++ right. rewrite Hw3, Hw4.
              assert (negb (gvn_root b <=? gvn_root a) = false) as Hr3.
              { apply negb_false_iff. apply Nat.leb_le. lia. }
              rewrite Hr3.
              assert (gvn_root b <? gvn_root a = false) as Hr4.
              { apply Nat.ltb_nlt. lia. }
              rewrite Hr4.
              apply negb_true_iff. apply Nat.ltb_nlt. lia.
           ++ left. apply negb_true_iff. apply Nat.ltb_nlt. lia.
        -- (* roots differ *)
           assert (gvn_root a <? gvn_root b = true) as Hr2.
           { apply Nat.ltb_lt. lia. }
           rewrite Hr2.
           right. rewrite Hw3, Hw4.
           assert (negb (gvn_root b <=? gvn_root a) = true) as Hr3.
           { apply negb_true_iff. apply Nat.leb_nle. lia. }
           rewrite Hr3. reflexivity.
      * (* root a > root b *)
        assert (negb (gvn_root a <=? gvn_root b) = true) as Hr1.
        { apply negb_true_iff. apply Nat.leb_nle. lia. }
        rewrite Hr1. left. reflexivity.
    + (* weight a < weight b *)
      assert (gvn_weight a <? gvn_weight b = true) as Hw2.
      { apply Nat.ltb_lt. lia. }
      rewrite Hw2.
      right.
      assert (negb (gvn_weight b <=? gvn_weight a) = true) as Hw3.
      { apply negb_true_iff. apply Nat.leb_nle. lia. }
      rewrite Hw3. reflexivity.
  - (* weight a > weight b *)
    left.
    assert (negb (gvn_weight a <=? gvn_weight b) = true) as Hw1.
    { apply negb_true_iff. apply Nat.leb_nle. lia. }
    rewrite Hw1. reflexivity.
Qed.

(** ** should_extend_payload model *)

(** Models the decision logic from proto_array_fork_choice.rs:1856-1927.
    Determines if a FULL node should be extended to its children's FULL path. *)

Record ExtendContext := mkExtendCtx {
  ec_inclusion_list_satisfied : bool;  (** Heze: IL constraint *)
  ec_ptc_timely : bool;               (** PTC quorum for timeliness *)
  ec_ptc_data_available : bool;       (** PTC quorum for blob availability *)
  ec_has_proposer_boost : bool;       (** is there a boosted block? *)
  ec_boost_parent_is_this : bool;     (** boosted block's parent is this node *)
  ec_boost_parent_full : bool;        (** boosted block's parent payload status is FULL *)
}.

Definition should_extend_payload (ctx : ExtendContext) : bool :=
  if negb (ec_inclusion_list_satisfied ctx) then false
  else if ec_ptc_timely ctx && ec_ptc_data_available ctx then true
  else if negb (ec_has_proposer_boost ctx) then true
  else if negb (ec_boost_parent_is_this ctx) then true
  else if ec_boost_parent_full ctx then true
  else false.

(** ** should_extend_payload properties *)

(** If IL is not satisfied, never extend (Heze safety). *)
Theorem extend_requires_il_satisfied :
  forall ctx,
    ec_inclusion_list_satisfied ctx = false ->
    should_extend_payload ctx = false.
Proof.
  intros ctx Hil. unfold should_extend_payload.
  rewrite Hil. simpl. reflexivity.
Qed.

(** PTC quorum (timely + data-available) always extends, regardless of boost. *)
Theorem ptc_quorum_always_extends :
  forall ctx,
    ec_inclusion_list_satisfied ctx = true ->
    ec_ptc_timely ctx = true ->
    ec_ptc_data_available ctx = true ->
    should_extend_payload ctx = true.
Proof.
  intros ctx Hil Ht Hda. unfold should_extend_payload.
  rewrite Hil. simpl. rewrite Ht, Hda. simpl. reflexivity.
Qed.

(** Without proposer boost, always extend (if IL satisfied). *)
Theorem no_boost_always_extends :
  forall ctx,
    ec_inclusion_list_satisfied ctx = true ->
    ec_has_proposer_boost ctx = false ->
    should_extend_payload ctx = true.
Proof.
  intros ctx Hil Hnb. unfold should_extend_payload.
  rewrite Hil. simpl.
  destruct (ec_ptc_timely ctx && ec_ptc_data_available ctx); [reflexivity|].
  rewrite Hnb. simpl. reflexivity.
Qed.

(** Boosted non-parent always extends (if IL satisfied). *)
Theorem boost_non_parent_extends :
  forall ctx,
    ec_inclusion_list_satisfied ctx = true ->
    ec_has_proposer_boost ctx = true ->
    ec_boost_parent_is_this ctx = false ->
    should_extend_payload ctx = true.
Proof.
  intros ctx Hil Hb Hnp. unfold should_extend_payload.
  rewrite Hil. simpl.
  destruct (ec_ptc_timely ctx && ec_ptc_data_available ctx); [reflexivity|].
  rewrite Hb. simpl. rewrite Hnp. simpl. reflexivity.
Qed.

(** Boosted parent with FULL status extends (if IL satisfied). *)
Theorem boost_full_parent_extends :
  forall ctx,
    ec_inclusion_list_satisfied ctx = true ->
    ec_has_proposer_boost ctx = true ->
    ec_boost_parent_is_this ctx = true ->
    ec_boost_parent_full ctx = true ->
    should_extend_payload ctx = true.
Proof.
  intros ctx Hil Hb Hp Hf. unfold should_extend_payload.
  rewrite Hil. simpl.
  destruct (ec_ptc_timely ctx && ec_ptc_data_available ctx); [reflexivity|].
  rewrite Hb. simpl. rewrite Hp. simpl. rewrite Hf. reflexivity.
Qed.

(** Boosted parent with non-FULL status does NOT extend
    (weak head protection — the critical safety case). *)
Theorem boost_non_full_parent_blocks :
  forall ctx,
    ec_inclusion_list_satisfied ctx = true ->
    ec_ptc_timely ctx = false ->
    ec_has_proposer_boost ctx = true ->
    ec_boost_parent_is_this ctx = true ->
    ec_boost_parent_full ctx = false ->
    should_extend_payload ctx = false.
Proof.
  intros ctx Hil Hnt Hb Hp Hnf. unfold should_extend_payload.
  rewrite Hil. simpl.
  rewrite Hnt. simpl.
  rewrite Hb. simpl. rewrite Hp. simpl. rewrite Hnf. reflexivity.
Qed.

(** ** Complete characterization of should_extend_payload *)

(** should_extend_payload returns true iff:
    IL is satisfied AND at least one of:
    - PTC quorum (timely + data-available)
    - no proposer boost
    - boost target's parent is not this node
    - boost target's parent has FULL status *)
Theorem should_extend_complete :
  forall ctx,
    should_extend_payload ctx = true <->
    (ec_inclusion_list_satisfied ctx = true /\
     (ec_ptc_timely ctx = true /\ ec_ptc_data_available ctx = true \/
      ec_has_proposer_boost ctx = false \/
      ec_boost_parent_is_this ctx = false \/
      ec_boost_parent_full ctx = true)).
Proof.
  intros ctx.
  destruct ctx as [il pt pda hpb bpit bpf].
  unfold should_extend_payload; simpl.
  destruct il, pt, pda, hpb, bpit, bpf; simpl;
    intuition discriminate.
Qed.

(** ** Gloas reorg resistance *)

(** Non-PENDING previous-slot nodes receive weight 0.
    Rust: proto_array_fork_choice.rs:1556-1559 *)

Definition gloas_effective_weight
  (status : PayloadStatus) (is_previous_slot : bool) (raw_weight : nat) : nat :=
  match status with
  | PENDING => raw_weight
  | _ => if is_previous_slot then 0 else raw_weight
  end.

Theorem reorg_resistance_empty :
  forall w, gloas_effective_weight EMPTY true w = 0.
Proof. reflexivity. Qed.

Theorem reorg_resistance_full :
  forall w, gloas_effective_weight FULL true w = 0.
Proof. reflexivity. Qed.

Theorem pending_keeps_weight :
  forall is_prev w, gloas_effective_weight PENDING is_prev w = w.
Proof. reflexivity. Qed.

Theorem non_previous_slot_keeps_weight :
  forall status w, gloas_effective_weight status false w = w.
Proof. destruct status; reflexivity. Qed.

(** ** Payload status transition well-formedness *)

Definition valid_transition (from to : PayloadStatus) (envelope_received : bool) : bool :=
  match from with
  | PENDING =>
    match to with
    | EMPTY => true
    | FULL => envelope_received
    | PENDING => false
    end
  | EMPTY | FULL =>
    match to with
    | PENDING => true
    | _ => false
    end
  end.

(** EMPTY is always reachable from PENDING. *)
Theorem empty_always_reachable :
  forall env, valid_transition PENDING EMPTY env = true.
Proof. reflexivity. Qed.

(** FULL requires envelope receipt. *)
Theorem full_requires_envelope :
  valid_transition PENDING FULL false = false.
Proof. reflexivity. Qed.

Theorem full_with_envelope :
  valid_transition PENDING FULL true = true.
Proof. reflexivity. Qed.

(** Children of decided nodes are always PENDING. *)
Theorem decided_to_pending :
  forall env, valid_transition EMPTY PENDING env = true /\
              valid_transition FULL PENDING env = true.
Proof. split; reflexivity. Qed.

(** No status loops. *)
Theorem no_self_transitions :
  forall env,
    valid_transition PENDING PENDING env = false /\
    valid_transition EMPTY EMPTY env = false /\
    valid_transition FULL FULL env = false.
Proof. intros. repeat split; reflexivity. Qed.

(** ** Transition alternation: the sequence always alternates PENDING <-> {EMPTY,FULL}. *)

(** After any valid transition from a decided state, we're in PENDING. *)
Theorem decided_implies_next_pending :
  forall from to env,
    (from = EMPTY \/ from = FULL) ->
    valid_transition from to env = true ->
    to = PENDING.
Proof.
  intros from to env [Hf|Hf]; subst; destruct to; simpl; intro H;
    try reflexivity; discriminate.
Qed.

(** After any valid transition from PENDING, we're in a decided state. *)
Theorem pending_implies_next_decided :
  forall to env,
    valid_transition PENDING to env = true ->
    to = EMPTY \/ to = FULL.
Proof.
  intros to env H. destruct to; simpl in H.
  - left. reflexivity.
  - right. reflexivity.
  - discriminate.
Qed.
