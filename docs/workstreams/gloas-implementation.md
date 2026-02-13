# workstream: gloas implementation

> status: **not started** | priority: 1 | branch: `gloas-dev` (not yet created)

## overview

Implement the Gloas (Glamsterdam consensus layer) fork in vibehouse. The centerpiece is EIP-7732 (ePBS).

## spec sources

- CL specs: https://github.com/ethereum/consensus-specs/tree/master/specs/gloas
- Engine API: https://github.com/ethereum/execution-apis/tree/main/src/engine
- EIP-7732: https://eips.ethereum.org/EIPS/eip-7732

## upstream status

- PR #8806: Gloas payload processing [WIP]
- PR #8815: Proposer lookahead endpoint
- Multiple EIP-7916 and EIP-8016 related PRs in progress

## our status

Not started. Need to:
1. Read the full gloas specs directory
2. Understand the ePBS data flow (proposer -> bid -> builder -> payload -> attestation)
3. Map spec changes to lighthouse crate structure
4. Begin with types and constants

## blockers

None yet.

## log

- 2026-02-13: workstream created, research phase
